//! 发送端：prepare-upload → 轮次循环 → 逐文件 upload。
//! 大文件走 AntifyBot 分块扩展（对端支持时）：16MiB 一块、每块独立 POST、
//! 断线后重新协商从断点续传；对端是官方 App / 老版本时自动回退整文件单发。
use crate::state::{now_ms, Device, SendProgress, Shared};
use anyhow::{bail, Context, Result};
use futures_util::StreamExt;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use tokio::io::{AsyncReadExt, AsyncSeekExt, SeekFrom};

/// 分块上传的默认块大小：WiFi 下每块秒级，HTTP 往返开销可忽略
pub const CHUNK_SIZE: usize = 16 * 1024 * 1024;

/// 实际生效的块大小（ANTIFY_CHUNK 环境变量可覆盖，如 "1M"/"4M"，测试用）
pub fn chunk_size() -> usize {
    static CACHE: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *CACHE.get_or_init(|| {
        std::env::var("ANTIFY_CHUNK")
            .ok()
            .and_then(|v| {
                let v = v.trim().to_ascii_lowercase();
                let (num, unit) = match v.as_bytes().last() {
                    Some(b'k') => (v.trim_end_matches('k'), 1024usize),
                    Some(b'm') => (v.trim_end_matches('m'), 1024 * 1024),
                    Some(b'g') => (v.trim_end_matches('g'), 1024 * 1024 * 1024),
                    _ => (v.as_str(), 1),
                };
                num.parse::<usize>().ok().map(|n| n.saturating_mul(unit))
            })
            .filter(|n| *n >= 64 * 1024)
            .unwrap_or(CHUNK_SIZE)
    })
}

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

/// 一次 prepare 的结果
struct Prepared {
    session_id: String,
    /// (文件序号, token)，按序号升序
    tokens: Vec<(usize, String)>,
}

/// 整轮退避（轮与轮之间）
const ROUND_BACKOFF_SECS: [u64; 2] = [2, 5];
/// prepare 撞上 409（会话被占）时的等待序列：总预算 ~67s，覆盖对端 60s 会话回收器
const PREPARE_BUSY_WAIT_SECS: [u64; 5] = [2, 5, 10, 20, 30];
/// 单块请求总超时（黑洞连接兜底；16MiB 在慢速 WiFi 下也只需十几秒）
const CHUNK_TIMEOUT_SECS: u64 = 180;

/// 向 target 发送一组文件；失败自动整轮重试（重新协商、断点续传），最多 3 轮。
/// 3 轮全败才报错——面板里消息置 ⚠，重试按钮继续从断点传
pub async fn send(state: &Shared, client: &reqwest::Client, target: &Device, items: Vec<SendItem>) -> Result<()> {
    if items.is_empty() {
        bail!("没有可发送的文件");
    }
    if state.send_cancel.load(Ordering::Relaxed) {
        bail!("发送已被取消");
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

    let base = target.base_url();

    // 进度心跳：每 300ms 把原子字节数刷进面板可见的快照。
    // fx/fo：当前文件的「起始 relay_bytes / 起始 offset（续传断点）」——
    // 多文件与断点续传下 sent 才不会串文件、不会从 0 跳
    let fx = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let fo = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    {
        let tick_state = state.clone();
        let fx = fx.clone();
        let fo = fo.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                let mut p = tick_state.sending.lock().await;
                if !p.active {
                    break;
                }
                let sent_this_file = tick_state
                    .relay_bytes
                    .load(Ordering::Relaxed)
                    .saturating_sub(fx.load(Ordering::Relaxed));
                p.sent = fo.load(Ordering::Relaxed) + sent_this_file;
            }
        });
    }

    state
        .log_event(format!("向「{}」发起传输（{} 个文件）", target.alias, items.len()))
        .await;

    let mut last_err: Option<anyhow::Error> = None;
    let mut last_session_id: Option<String> = None;

    'rounds: for round in 0..3 {
        if round > 0 {
            if state.send_cancel.load(Ordering::Relaxed) {
                break;
            }
            let wait = ROUND_BACKOFF_SECS[(round - 1).min(ROUND_BACKOFF_SECS.len() - 1)];
            state
                .log_event(format!("第 {round} 次尝试失败，{wait} 秒后自动重试（可断点续传）"))
                .await;
            tokio::time::sleep(std::time::Duration::from_secs(wait)).await;
        }

        // ---- prepare-upload（409 = 会话被占：清自己的僵尸会话 + 退避等让位）----
        let mut prepared: Option<Prepared> = None;
        for (attempt, wait) in std::iter::once(0u64)
            .chain(PREPARE_BUSY_WAIT_SECS.iter().copied())
            .enumerate()
        {
            if attempt > 0 {
                if state.send_cancel.load(Ordering::Relaxed) {
                    break 'rounds;
                }
                tokio::time::sleep(std::time::Duration::from_secs(wait)).await;
            }
            let resp = match client
                .post(format!("{base}/api/localsend/v2/prepare-upload"))
                .timeout(std::time::Duration::from_secs(10))
                .json(&body)
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    // 对端不可达：本 ROUND 直接失败，交给轮次重试
                    last_err = Some(anyhow::anyhow!("prepare-upload 联系不上对端: {e}"));
                    continue 'rounds;
                }
            };
            match resp.status().as_u16() {
                204 => {
                    // 官方协议的「拒收」：对方主动取消/拒绝，不重试
                    cleanup_progress(state).await;
                    state
                        .log_event(format!("「{}」拒收了本次传输（对方可能取消了接收）", target.alias))
                        .await;
                    bail!("「{}」拒收了本次传输", target.alias);
                }
                409 => {
                    // 会话被占：多半是自己上一轮的僵尸会话 → 带旧 id 请对端清掉
                    //（不匹配时对端 no-op，绝不误伤别人的会话）；然后退避等 60s 回收器
                    if attempt == 0 {
                        if let Some(id) = &last_session_id {
                            let _ = client
                                .post(format!("{base}/api/localsend/v2/cancel?sessionId={id}"))
                                .timeout(std::time::Duration::from_secs(10))
                                .send()
                                .await;
                        }
                    }
                    continue;
                }
                s if (200..300).contains(&s) => {
                    match parse_prepare(resp).await {
                        Ok(p) => {
                            prepared = Some(p);
                            break;
                        }
                        Err(e) => {
                            last_err = Some(e);
                            continue 'rounds;
                        }
                    }
                }
                s => {
                    last_err = Some(anyhow::anyhow!("prepare-upload 返回 {s}"));
                    continue 'rounds;
                }
            }
        }
        let Some(prepared) = prepared else {
            last_err = Some(anyhow::anyhow!(
                "对方会话一直被占用（可能正在接收其他传输）"
            ));
            continue;
        };
        last_session_id = Some(prepared.session_id.clone());

        // ---- 逐文件上传 ----
        // 对端能力探测结果：Some(true)=支持分块续传；Some(false)=官方/老版本（本轮全单发）
        let mut chunked_capable: Option<bool> = None;

        for (idx, token) in &prepared.tokens {
            if state.send_cancel.load(Ordering::Relaxed) {
                break 'rounds;
            }
            let item = &items[*idx];
            let upload_base = format!(
                "{base}/api/localsend/v2/upload?sessionId={}&fileId=f{idx}&token={token}",
                prepared.session_id
            );

            let use_chunked = item.size > chunk_size() as u64 && item.sha256.is_some();
            let mut resume_offset: u64 = 0;
            if use_chunked && chunked_capable != Some(false) {
                match probe_resume(client, &base, &prepared.session_id, *idx, token, item).await {
                    Ok(Some(off)) => {
                        chunked_capable = Some(true);
                        if off >= item.size {
                            // 对端已有完整数据（上次会话收满但没收尾，resume-info 顺手收了尾）
                            state
                                .log_event(format!(
                                    "「{}」已有完整的 {}，跳过上传",
                                    target.alias, item.file_name
                                ))
                                .await;
                            continue;
                        }
                        if off > 0 {
                            state
                                .log_event(format!(
                                    "断点续传：{} 从 {}/{} 字节处继续",
                                    item.file_name, off, item.size
                                ))
                                .await;
                        }
                        resume_offset = off;
                    }
                    Ok(None) => chunked_capable = Some(false),
                    Err(e) => {
                        // 会话失效（对端重启）/网络问题：本轮结束，整轮重来
                        last_err = Some(e);
                        continue 'rounds;
                    }
                }
            }

            // 进度基准：本文件从 resume_offset 起算
            fx.store(state.relay_bytes.load(Ordering::Relaxed), Ordering::Relaxed);
            fo.store(resume_offset, Ordering::Relaxed);
            set_progress(state, &target.alias, &item.file_name, resume_offset, item.size, true).await;

            let result = match &item.source {
                Source::Text(data) => {
                    send_text(client, &upload_base, item, data.clone()).await
                }
                Source::Path(path) => {
                    if resume_offset > 0 || (use_chunked && chunked_capable == Some(true)) {
                        send_chunked(state, client, &upload_base, path, item, resume_offset)
                            .await
                    } else {
                        // 官方/老版本对端，或小文件：整文件流式单发（今天的行为）
                        send_streamed(state, client, &upload_base, path, item).await
                    }
                }
            };
            match result {
                Ok(()) => {
                    state
                        .log_event(format!("已送达「{}」：{}", target.alias, item.file_name))
                        .await;
                }
                Err(e) => {
                    last_err = Some(e);
                    continue 'rounds;
                }
            }
        }

        // 全部送达
        cleanup_progress(state).await;
        return Ok(());
    }

    // 轮次耗尽（或中途取消）
    cleanup_progress(state).await;
    if state.send_cancel.load(Ordering::Relaxed) {
        bail!("发送已取消（重试可从断点续传）");
    }
    match last_err {
        Some(e) => Err(e.context(format!("向「{}」传输失败（重试可断点续传）", target.alias))),
        None => bail!("发送已取消"),
    }
}

async fn cleanup_progress(state: &Shared) {
    // 心跳线程靠 active=false 自行退出
    *state.sending.lock().await = SendProgress {
        active: false,
        ..Default::default()
    };
}

async fn parse_prepare(resp: reqwest::Response) -> Result<Prepared> {
    let parsed: PrepareResponse = resp.json().await.context("解析 prepare-upload 响应失败")?;
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
    Ok(Prepared {
        session_id: parsed.session_id,
        tokens,
    })
}

/// 探测对端续传状态。
/// Ok(None) = 对端不支持（404：官方 App / 老版本）→ 整文件单发；
/// Ok(Some(offset)) = 支持，已收 offset 字节
async fn probe_resume(
    client: &reqwest::Client,
    base: &str,
    session_id: &str,
    idx: usize,
    token: &str,
    item: &SendItem,
) -> Result<Option<u64>> {
    let url = format!(
        "{base}/api/antify/v1/resume-info?sessionId={session_id}&fileId=f{idx}&token={token}"
    );
    let resp = client
        .get(&url)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
        .context("查询续传状态失败")?;
    if resp.status().as_u16() == 404 {
        return Ok(None);
    }
    if !resp.status().is_success() {
        // 403 等：会话失效（对端重启过）——交给整轮重来
        bail!("resume-info 返回 {}", resp.status());
    }
    let v: serde_json::Value = resp.json().await.context("resume-info 响应异常")?;
    let offset = v.get("offset").and_then(|x| x.as_u64()).unwrap_or(0);
    let size = v.get("size").and_then(|x| x.as_u64()).unwrap_or(u64::MAX);
    let sha = v.get("sha256").and_then(|x| x.as_str()).unwrap_or("");
    // 确实是同一文件的进度才续；对不上当不支持（从 0 单发最稳）
    let mine = item.sha256.as_deref().unwrap_or("");
    if size != item.size || !sha.eq_ignore_ascii_case(mine) {
        return Ok(None);
    }
    Ok(Some(offset.min(item.size)))
}

/// 文字消息：小负载单发（15s 足够）
async fn send_text(
    client: &reqwest::Client,
    upload_base: &str,
    item: &SendItem,
    data: Vec<u8>,
) -> Result<()> {
    let resp = client
        .post(upload_base)
        .timeout(std::time::Duration::from_secs(15))
        .header("Content-Type", &item.mime)
        .body(data)
        .send()
        .await
        .with_context(|| format!("上传 {} 失败", item.file_name))?;
    if !resp.status().is_success() {
        bail!("上传 {} 返回 {}", item.file_name, resp.status());
    }
    Ok(())
}

/// 整文件流式单发（官方/老版本对端、小文件）：今天的行为，不设总超时
/// （大文件传输可能很久，连接层超时已兜底）
async fn send_streamed(
    state: &Shared,
    client: &reqwest::Client,
    upload_base: &str,
    path: &std::path::Path,
    item: &SendItem,
) -> Result<()> {
    let file = tokio::fs::File::open(path)
        .await
        .with_context(|| format!("打开 {}", path.display()))?;
    let stream = tokio_util::io::ReaderStream::new(file);
    let counter_state = state.clone(); // 整个传输克隆一次 Arc
    let counted = stream.map(move |chunk| {
        if let Ok(ref b) = chunk {
            counter_state
                .relay_bytes
                .fetch_add(b.len() as u64, Ordering::Relaxed);
        }
        chunk
    });
    let resp = client
        .post(upload_base)
        .header("Content-Type", &item.mime)
        .body(reqwest::Body::wrap_stream(counted))
        .send()
        .await
        .with_context(|| format!("上传 {} 失败", item.file_name))?;
    if !resp.status().is_success() {
        bail!("上传 {} 返回 {}", item.file_name, resp.status());
    }
    Ok(())
}

/// 分块上传（AntifyBot 对端）：从 resume_offset 起，16MiB 一块逐块 POST。
/// 每块 3 次重试；对端 409（进度失步）时按其回报的 offset 重对齐；块重试耗尽
/// 报错给整轮重来（重新 prepare + 重新探测 = 自动断点续传）
async fn send_chunked(
    state: &Shared,
    client: &reqwest::Client,
    upload_base: &str,
    path: &std::path::Path,
    item: &SendItem,
    resume_offset: u64,
) -> Result<()> {
    let Source::Path(_) = &item.source else {
        bail!("分块上传只支持文件");
    };
    let chunk = chunk_size();
    let total = item.size;
    let mut offset = resume_offset.min(total);
    let mut file = tokio::fs::File::open(path)
        .await
        .with_context(|| format!("打开 {}", path.display()))?;
    let mut buf = vec![0u8; chunk];

    while offset < total {
        if state.send_cancel.load(Ordering::Relaxed) {
            bail!("发送已取消");
        }
        // 源文件在发送期间被改（长度变了）：中止——内容变化由块 hash 在对端兜底
        let len_now = file.metadata().await.map(|m| m.len()).unwrap_or(u64::MAX);
        if len_now != total {
            bail!("源文件在发送过程中发生了变化，已中止（请重新发起发送）");
        }
        let take = ((total - offset).min(chunk as u64)) as usize;
        file.seek(SeekFrom::Start(offset)).await?;
        let mut filled = 0usize;
        while filled < take {
            let n = file.read(&mut buf[filled..take]).await?;
            if n == 0 {
                bail!("源文件读取不足（发送过程中被截短？）");
            }
            filled += n;
        }
        let chunk_sha = crate::resume::sha256_hex(&buf[..take]);
        let url = format!("{upload_base}&offset={offset}&len={take}");

        let mut chunk_failed: Option<String> = None;
        // 409 重对齐后不能按本块的 take 推进 offset（那是旧起点的长度，
        // 会再次越过对端 received → 无限 409），改为 continue 按新 offset 重算
        let mut realigned = false;
        for attempt in 0..3 {
            if attempt > 0 {
                if state.send_cancel.load(Ordering::Relaxed) {
                    bail!("发送已取消");
                }
                tokio::time::sleep(std::time::Duration::from_secs(attempt as u64)).await;
            }
            match client
                .post(&url)
                .timeout(std::time::Duration::from_secs(CHUNK_TIMEOUT_SECS))
                .header("Content-Type", &item.mime)
                .header("X-Chunk-SHA256", &chunk_sha)
                .body(buf[..take].to_vec())
                .send()
                .await
            {
                Ok(r) if r.status().is_success() => {
                    chunk_failed = None;
                    break;
                }
                Ok(r) if r.status().as_u16() == 409 => {
                    // 对端进度与我们有分歧（上一块其实没写成）：按其回报回退重对齐。
                    // 只接受严格更小的断点（保证收敛）；不小于当前 offset 的回报
                    // 无法推进，视为失败交给轮次重试
                    let v: serde_json::Value = r.json().await.unwrap_or_default();
                    let remote = v.get("offset").and_then(|x| x.as_u64()).unwrap_or(0);
                    if remote < offset {
                        offset = remote;
                        realigned = true;
                        chunk_failed = None;
                    } else {
                        chunk_failed = Some(format!("对端 409 回报了异常断点 {remote}"));
                    }
                    break;
                }
                Ok(r) => chunk_failed = Some(format!("对端返回 {}", r.status())),
                Err(e) => chunk_failed = Some(format!("{e}")),
            }
        }
        if let Some(e) = chunk_failed {
            bail!("上传 {} 字节 {offset}..{} 失败：{e}", item.file_name, offset + take as u64);
        }
        if realigned {
            continue;
        }

        offset += take as u64;
        state.relay_bytes.fetch_add(take as u64, Ordering::Relaxed);
    }
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

/// CLI/面板中转用：从磁盘构造 SendItem（一次遍历同时算出总 SHA-256 与逐块 hash）
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
