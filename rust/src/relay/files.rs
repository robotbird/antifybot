use super::*;
use futures_util::StreamExt;
use tokio::io::AsyncWriteExt;

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
        while let Some(chunk) = tokio::time::timeout(Duration::from_secs(60), stream.next()).await?
        {
            let chunk = chunk?;
            size = size.saturating_add(chunk.len() as u64);
            ensure!(size <= MAX_FILE && size <= expected, "文件大小超出声明值");
            file.write_all(&chunk).await?;
        }
        ensure!(size == expected, "文件接收不完整");
        file.sync_all().await?;
        Ok::<(), anyhow::Error>(())
    };
    write.await
}
