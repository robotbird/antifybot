//! 发送端：prepare-upload → 逐文件 upload（流式，不占内存）
use crate::state::{now_ms, Device, SendProgress, Shared};
use anyhow::{bail, Context, Result};
use futures_util::StreamExt;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use tokio::io::AsyncReadExt;

pub struct SendItem {
    pub file_name: String,
    pub size: u64,
    pub mime: String,
    pub sha256: Option<String>,
    pub source: Source,
}

pub enum Source {
    Path(PathBuf),
    Text(Vec<u8>),
}

#[derive(Deserialize)]
struct PrepareResponse {
    #[serde(alias = "sessionId")]
    session_id: String,
    /// v2: { fileId: token }；老实现可能是 { fileId: {token} } —— 两种都收
    files: serde_json::Value,
}

/// 向 target 发送一组文件；identity 用于构造 info 块
pub async fn send(state: &Shared, client: &reqwest::Client, target: &Device, items: Vec<SendItem>) -> Result<()> {
    if items.is_empty() {
        bail!("没有可发送的文件");
    }

    // 构造 prepare-upload 请求
    let mut files = serde_json::Map::new();
    for (i, item) in items.iter().enumerate() {
        let mut f = serde_json::json!({
            "id": format!("f{i}"),
            "fileName": item.file_name,
            "size": item.size,
            "fileType": item.mime,
            "sha256": item.sha256,
        });
        // 文字消息照官方格式把正文嵌进 preview，接收端据此区分消息与普通 .txt 文件
        if let Source::Text(bytes) = &item.source {
            f["preview"] = serde_json::json!(String::from_utf8_lossy(bytes).trim());
        }
        files.insert(format!("f{i}"), f);
    }
    let body = serde_json::json!({
        "info": state.identity.register_json(),
        "files": files,
    });

    state
        .log_event(format!("向「{}」发起传输（{} 个文件）", target.alias, items.len()))
        .await;

    let url = format!("{}/api/localsend/v2/prepare-upload", target.base_url());
    let resp = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .with_context(|| format!("prepare-upload 请求失败（{}）", url))?;
    let status = resp.status();
    if status.as_u16() == 204 {
        state.log_event(format!("「{}」拒收（204，无文件需要传输）", target.alias)).await;
        return Ok(());
    }
    if !status.is_success() {
        bail!("prepare-upload 返回 {status}");
    }
    let parsed: PrepareResponse = resp
        .json()
        .await
        .context("解析 prepare-upload 响应失败")?;

    // 解析 token（兼容字符串与 {token} 对象两种形态）
    let mut tokens: Vec<(usize, String)> = Vec::new();
    if let Some(map) = parsed.files.as_object() {
        for (k, v) in map {
            let idx: usize = k.trim_start_matches('f').parse().unwrap_or(usize::MAX);
            let token = v
                .as_str()
                .map(str::to_string)
                .or_else(|| v.get("token").and_then(|t| t.as_str()).map(str::to_string));
            if let Some(t) = token {
                if idx != usize::MAX {
                    tokens.push((idx, t));
                }
            }
        }
    }
    if tokens.is_empty() {
        bail!("对方未接受任何文件");
    }
    tokens.sort_by_key(|(i, _)| *i);

    let base = target.base_url();
    // 进度心跳：每 300ms 把原子字节数刷进面板可见的快照
    let tick_state = state.clone();
    let byte_base = state.relay_bytes.load(std::sync::atomic::Ordering::Relaxed);
    let ticker = tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            let mut p = tick_state.sending.lock().await;
            if !p.active {
                break;
            }
            p.sent = tick_state
                .relay_bytes
                .load(std::sync::atomic::Ordering::Relaxed)
                .saturating_sub(byte_base);
        }
    });
    for (idx, token) in tokens {
        let item = &items[idx];
        set_progress(state, &target.alias, &item.file_name, 0, item.size, true).await;

        let upload_url = format!(
            "{base}/api/localsend/v2/upload?sessionId={}&fileId=f{idx}&token={token}",
            parsed.session_id
        );
        let req = client.post(&upload_url);
        let resp = match &item.source {
            Source::Text(data) => req
                .header("Content-Type", &item.mime)
                .body(data.clone())
                .send()
                .await,
            Source::Path(path) => {
                let file = tokio::fs::File::open(path)
                    .await
                    .with_context(|| format!("打开 {}", path.display()))?;
                let stream = tokio_util::io::ReaderStream::new(file);
                let counter_state = state.clone(); // 整个传输克隆一次 Arc
                let counted = stream.map(move |chunk| {
                    if let Ok(ref b) = chunk {
                        counter_state
                            .relay_bytes
                            .fetch_add(b.len() as u64, std::sync::atomic::Ordering::Relaxed);
                    }
                    chunk
                });
                req.header("Content-Type", &item.mime)
                    .body(reqwest::Body::wrap_stream(counted))
                    .send()
                    .await
            }
        };
        match resp {
            Ok(r) if r.status().is_success() => {
                state
                    .log_event(format!("已送达「{}」：{}", target.alias, item.file_name))
                    .await;
            }
            Ok(r) => bail!("上传 {} 返回 {}", item.file_name, r.status()),
            Err(e) => bail!("上传 {} 失败: {e}", item.file_name),
        }
    }

    ticker.abort();
    set_progress(state, "", "", 0, 0, false).await;
    Ok(())
}

async fn set_progress(state: &Shared, alias: &str, name: &str, sent: u64, total: u64, active: bool) {
    *state.sending.lock().await = SendProgress {
        active,
        target_alias: alias.to_string(),
        file_name: name.to_string(),
        sent,
        total,
        started_at: now_ms(),
    };
}

/// CLI/面板中转用：从磁盘构造 SendItem（顺带算 SHA-256）
pub async fn item_from_path(path: PathBuf) -> Result<SendItem> {
    let meta = tokio::fs::metadata(&path).await.with_context(|| format!("读取 {}", path.display()))?;
    if !meta.is_file() {
        bail!("{} 不是常规文件", path.display());
    }
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".to_string());
    let mime = mime_from_name(&file_name);
    let mut f = tokio::fs::File::open(&path).await?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 512 * 1024];
    loop {
        let n = f.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(SendItem {
        file_name,
        size: meta.len(),
        mime,
        sha256: Some(hex::encode(hasher.finalize())),
        source: Source::Path(path),
    })
}

pub fn item_from_text(text: String) -> SendItem {
    // 官方客户端的「发送文本」= <uuid>.txt + text/plain + preview 内嵌正文，照此构造
    SendItem {
        file_name: format!("{}.txt", uuid::Uuid::new_v4()),
        size: text.len() as u64,
        mime: "text/plain".to_string(),
        sha256: None,
        source: Source::Text(text.into_bytes()),
    }
}

fn mime_from_name(name: &str) -> String {
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "heic" => "image/heic",
        "svg" => "image/svg+xml",
        "mp4" => "video/mp4",
        "mov" => "video/quicktime",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        "txt" => "text/plain",
        "json" => "application/json",
        _ => "application/octet-stream",
    }
    .to_string()
}
