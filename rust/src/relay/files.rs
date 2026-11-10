use super::*;
use futures_util::StreamExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub(super) const MAX_FILE: u64 = 100 * 1024 * 1024;
pub(super) fn safe_name(name: &str) -> Result<()> {
    ensure!(
        !name.is_empty()
            && name.len() <= 255
            && name != "."
            && name != ".."
            && !name
                .chars()
                .any(|c| c.is_control() || c == '/' || c == '\\'),
        "文件名不合法"
    );
    Ok(())
}
pub(super) fn image_mime(name: &str) -> Option<&'static str> {
    match name
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "webp" => Some("image/webp"),
        "gif" => Some("image/gif"),
        _ => None,
    }
}
pub(super) async fn stage(path: &Path, expected: u64, body: axum::body::Body) -> Result<()> {
    let write = async {
        let mut file = tokio::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(path)
            .await?;
        let mut stream = body.into_data_stream();
        let mut size = 0u64;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            size = size.saturating_add(chunk.len() as u64);
            ensure!(size <= MAX_FILE && size <= expected, "文件大小超出声明值");
            file.write_all(&chunk).await?;
        }
        ensure!(size == expected, "文件接收不完整");
        file.sync_all().await?;
        Ok::<(), anyhow::Error>(())
    };
    tokio::time::timeout(Duration::from_secs(300), write).await?
}

struct ProgressGuard(Shared, String);
impl Drop for ProgressGuard {
    fn drop(&mut self) {
        if let Ok(mut progress) = self.0.sending.try_lock() {
            if progress.file_name == self.1 && progress.target_alias == "中转" {
                *progress = Default::default();
            }
        }
    }
}
pub(super) async fn upload(
    state: &Shared,
    cfg: &Settings,
    client: &reqwest::Client,
    item: &RelayPending,
) -> Result<FileInfo> {
    let path = Path::new(&item.src_path);
    let meta = tokio::fs::metadata(path).await?;
    if !meta.is_file() || meta.len() != item.size || meta.len() > MAX_FILE {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "源文件已改变，请重新发送",
        )
        .into());
    }
    let mut file = tokio::fs::File::open(path).await?;
    let mut hash = Sha256::new();
    let mut buf = vec![0; 128 * 1024];
    loop {
        let n = file.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
    }
    let digest = hex::encode(hash.finalize());
    let file = tokio::fs::File::open(path).await?;
    state.send_cancel.store(false, Ordering::Relaxed);
    *state.sending.lock().await = crate::state::SendProgress {
        active: true,
        target_alias: "中转".into(),
        file_name: item.name.clone(),
        sent: 0,
        total: item.size,
        started_at: now_ms(),
    };
    let _guard = ProgressGuard(state.clone(), item.name.clone());
    let counter = state.clone();
    let stream = tokio_util::io::ReaderStream::new(file).map(move |chunk| {
        if counter.send_cancel.load(Ordering::Relaxed) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "发送已取消",
            ));
        }
        if let Ok(ref bytes) = chunk {
            if let Ok(mut p) = counter.sending.try_lock() {
                p.sent = p.sent.saturating_add(bytes.len() as u64);
            }
        }
        chunk
    });
    let info: FileInfo = client
        .post(format!("{}/v1/files", cfg.server_url))
        .bearer_auth(&cfg.token)
        .query(&[
            ("to", item.recipient.as_str()),
            ("name", item.name.as_str()),
        ])
        .header("Content-Length", item.size)
        .header("X-File-SHA256", &digest)
        .timeout(Duration::from_secs(300))
        .body(reqwest::Body::wrap_stream(stream))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(
        info.sha256 == digest && info.size == item.size,
        "服务端文件校验结果不一致"
    );
    Ok(info)
}

pub(super) async fn download(
    state: &Shared,
    cfg: &Settings,
    client: &reqwest::Client,
    message_id: &str,
    info: &FileInfo,
) -> Result<String> {
    let id = uuid::Uuid::parse_str(message_id)?.to_string();
    uuid::Uuid::parse_str(&info.id)?;
    safe_name(&info.name)?;
    ensure!(
        info.size <= MAX_FILE
            && info.sha256.len() == 64
            && info.sha256.bytes().all(|b| b.is_ascii_hexdigit()),
        "附件大小或摘要不合法"
    );
    let relative = PathBuf::from(format!("Relay-{}", &cfg.scope()[..8]))
        .join(id)
        .join(&info.name);
    let root = state.download_dir.read().await.clone();
    let destination = root.join(&relative);
    tokio::fs::create_dir_all(destination.parent().context("下载路径无效")?).await?;
    if destination.exists() && verify(&destination, info).await? {
        return Ok(relative.to_string_lossy().into_owned());
    }
    ensure!(
        !destination.exists(),
        "已存在同名但内容不同的文件，请移走后重试"
    );
    let part = destination.parent().unwrap().join(".download.part");
    let result = async {
        let response = client
            .get(format!("{}/v1/files/{}", cfg.server_url, info.id))
            .bearer_auth(&cfg.token)
            .timeout(Duration::from_secs(300))
            .send()
            .await?
            .error_for_status()?;
        let mut stream = response.bytes_stream();
        let mut file = tokio::fs::File::create(&part).await?;
        let mut got = 0u64;
        let mut hash = Sha256::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            got = got.saturating_add(chunk.len() as u64);
            ensure!(got <= info.size, "附件大小超过声明值");
            hash.update(&chunk);
            file.write_all(&chunk).await?;
        }
        ensure!(
            got == info.size && hex::encode(hash.finalize()).eq_ignore_ascii_case(&info.sha256),
            "附件 SHA-256 校验失败"
        );
        file.sync_all().await?;
        drop(file);
        tokio::fs::rename(&part, &destination).await?;
        Ok::<(), anyhow::Error>(())
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&part).await;
    }
    result?;
    Ok(relative.to_string_lossy().into_owned())
}
async fn verify(path: &Path, info: &FileInfo) -> Result<bool> {
    if tokio::fs::metadata(path).await?.len() != info.size {
        return Ok(false);
    }
    let mut file = tokio::fs::File::open(path).await?;
    let mut hash = Sha256::new();
    let mut buf = vec![0; 128 * 1024];
    loop {
        let n = file.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
    }
    Ok(hex::encode(hash.finalize()).eq_ignore_ascii_case(&info.sha256))
}
