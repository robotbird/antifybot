use super::rtc::{Wire, PACKET};
use super::*;
use crate::state::SendProgress;
use tokio::fs::{self, File, OpenOptions};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

pub(in crate::relay) const MAX_FILE: u64 = 16 * 1024 * 1024 * 1024;
const WINDOW: u64 = 1024 * 1024;
const IN_FLIGHT: u64 = 4 * WINDOW;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(in crate::relay) enum Payload {
    Text {
        text: String,
    },
    File {
        name: String,
        size: u64,
        sha256: String,
    },
}

#[derive(Deserialize, Serialize)]
struct Transfer {
    id: String,
    payload: Payload,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    file_type: Option<String>,
}

#[derive(Deserialize, Serialize)]
enum Reply {
    Offset(u64),
    Delivered,
    Rejected(String),
}

fn file_type(name: &str) -> String {
    Path::new(name)
        .extension()
        .and_then(|extension| extension.to_str())
        .filter(|extension| {
            !extension.is_empty()
                && extension.len() <= 16
                && extension.bytes().all(|b| b.is_ascii_alphanumeric())
        })
        .map_or_else(
            || "application/octet-stream".into(),
            str::to_ascii_uppercase,
        )
}

pub(in crate::relay) async fn hash(file: &mut File) -> Result<String> {
    file.seek(std::io::SeekFrom::Start(0)).await?;
    let mut digest = ring::digest::Context::new(&ring::digest::SHA256);
    let mut buf = vec![0; 256 * 1024];
    loop {
        let n = file.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        digest.update(&buf[..n]);
    }
    Ok(hex::encode(digest.finish().as_ref()))
}

pub(super) async fn open_source(item: &RelayPending, payload: &Payload) -> Result<Option<File>> {
    let source = match payload {
        Payload::Text { .. } => None,
        Payload::File { size, sha256, .. } => {
            let mut file = File::open(&item.src_path)
                .await
                .context("源文件已移动或删除，请重新选择")?;
            ensure!(
                file.metadata().await?.len() == *size && hash(&mut file).await? == *sha256,
                "源文件已修改，请创建新的发送任务"
            );
            Some(file)
        }
    };
    Ok(source)
}

pub(super) async fn send(
    state: &Shared,
    wire: &Wire,
    item: &RelayPending,
    payload: Payload,
    mut source: Option<File>,
) -> Result<()> {
    let file_type = match &payload {
        Payload::File { name, .. } => Some(file_type(name)),
        Payload::Text { .. } => None,
    };
    let request = Transfer {
        id: item.remote_id.clone(),
        payload,
        file_type,
    };
    wire.send(&request).await?;
    let first: Reply = wire.recv().await?;
    match first {
        Reply::Delivered => return wire.send(&true).await,
        Reply::Rejected(reason) => bail!("{reason}"),
        Reply::Offset(_) => {}
    }
    let Payload::File { name, size, .. } = &request.payload else {
        bail!("文字回执无效");
    };
    let Reply::Offset(offset) = first else {
        unreachable!()
    };
    ensure!(
        offset <= *size && (offset == *size || offset % WINDOW == 0),
        "接收端偏移无效"
    );
    if offset > 0 {
        state
            .log_event(format!("点对点续传起点 {offset} 字节"))
            .await;
    }
    let file = source.as_mut().context("缺少源文件")?;
    file.seek(std::io::SeekFrom::Start(offset)).await?;
    *state.relay.progress.lock().unwrap() = SendProgress {
        active: true,
        target_alias: if wire.is_relay() { "HTTPS" } else { "P2P" }.into(),
        file_name: name.clone(),
        sent: offset,
        total: *size,
        started_at: now_ms(),
    };
    let size = *size;
    let (confirmed, mut credit) = tokio::sync::watch::channel(offset);
    let write = async {
        let mut buffer = vec![0; WINDOW as usize];
        let mut sent = offset;
        while sent < size {
            let len = (size - sent).min(WINDOW);
            credit
                .wait_for(|saved| sent + len <= *saved + IN_FLIGHT)
                .await
                .map_err(|_| super::rtc::Interrupted)?;
            file.read_exact(&mut buffer[..len as usize]).await?;
            for packet in buffer[..len as usize].chunks(PACKET) {
                ensure!(!state.send_cancel.load(Ordering::Relaxed), "发送已取消");
                wire.write(packet).await?;
            }
            sent += len;
        }
        Ok(())
    };
    let acknowledge = async {
        let mut saved = offset;
        let mut checkpoint_started = std::time::Instant::now();
        while saved < size {
            let expected = size.min(saved + WINDOW);
            match wire.recv().await? {
                Reply::Offset(next) if next == expected => {
                    let elapsed = checkpoint_started.elapsed();
                    if saved == offset || elapsed >= Duration::from_secs(2) {
                        state
                            .log_event(format!(
                                "点对点确认已写入 {next} / {size} 字节，本段耗时 {:.2} 秒",
                                elapsed.as_secs_f64()
                            ))
                            .await;
                    }
                    checkpoint_started = std::time::Instant::now();
                    saved = next;
                    state.relay.progress.lock().unwrap().sent = saved;
                    confirmed.send_replace(saved);
                }
                Reply::Rejected(reason) => bail!("{reason}"),
                _ => bail!("接收端未确认写入进度"),
            }
        }
        match wire.recv().await? {
            Reply::Delivered => wire.send(&true).await,
            Reply::Rejected(reason) => bail!("{reason}"),
            _ => bail!("对端未确认文件校验"),
        }
    };
    tokio::try_join!(write, acknowledge)?;
    Ok(())
}

pub(super) async fn receive(
    state: &Shared,
    cfg: &Settings,
    peer: &DirectDevice,
    wire: &Wire,
) -> Result<()> {
    let request: Transfer = wire.recv().await?;
    let result = receive_inner(state, cfg, peer, wire, request).await;
    if let Err(ref error) = result {
        let _ = wire.send(&Reply::Rejected(error.to_string())).await;
    }
    result
}

struct IncomingTransfer<'a> {
    state: &'a Shared,
    key: (String, String),
}

impl IncomingTransfer<'_> {
    fn set_progress(&self, progress: ReceiveProgress) {
        if let Some(entry) = self
            .state
            .relay
            .receiving
            .lock()
            .unwrap()
            .get_mut(&self.key)
        {
            *entry = Some(progress);
        }
    }

    fn confirm(&self, got: u64) {
        if let Some(Some(progress)) = self
            .state
            .relay
            .receiving
            .lock()
            .unwrap()
            .get_mut(&self.key)
        {
            progress.got = got;
            progress.verifying = got == progress.total;
        }
    }
}

impl Drop for IncomingTransfer<'_> {
    fn drop(&mut self) {
        self.state.relay.receiving.lock().unwrap().remove(&self.key);
    }
}

async fn receive_inner(
    state: &Shared,
    cfg: &Settings,
    peer: &DirectDevice,
    wire: &Wire,
    request: Transfer,
) -> Result<()> {
    let id = uuid::Uuid::parse_str(&request.id)?.to_string();
    let scope = format!("p2p:{}:{}", cfg.scope(), peer.fingerprint);
    let key = (scope.clone(), id.clone());
    {
        use std::collections::hash_map::Entry;
        let mut receiving = state.relay.receiving.lock().unwrap();
        let Entry::Vacant(entry) = receiving.entry(key.clone()) else {
            bail!("该传输已在接收，请稍后重试");
        };
        entry.insert(None);
    }
    let incoming = IncomingTransfer { state, key };
    if state.db.relay_seen(&scope, &id)? {
        return delivered(wire).await;
    }
    let mut message = ChatMsg {
        peer: peer.fingerprint.clone(),
        peer_alias: peer.alias.clone(),
        at: now_ms(),
        ..Default::default()
    };
    match &request.payload {
        Payload::Text { text } => {
            ensure!(!text.is_empty() && text.len() <= 32 * 1024, "文字大小无效");
            message.kind = "text".into();
            message.text = text.clone();
        }
        Payload::File { name, size, sha256 } => {
            files::safe_name(name)?;
            ensure!(
                *size <= MAX_FILE
                    && sha256.len() == 64
                    && sha256.bytes().all(|b| b.is_ascii_hexdigit()),
                "文件属性无效"
            );
            message.kind = "file".into();
            message.name = name.clone();
            message.size = *size;
            incoming.set_progress(ReceiveProgress {
                peer: peer.fingerprint.clone(),
                sender: peer.alias.clone(),
                name: name.clone(),
                file_type: request
                    .file_type
                    .as_deref()
                    .filter(|kind| {
                        !kind.is_empty()
                            && kind.len() <= 32
                            && kind.bytes().all(|b| b.is_ascii_graphic())
                    })
                    .map(str::to_owned)
                    .unwrap_or_else(|| file_type(name)),
                verifying: false,
                relayed: wire.is_relay(),
                got: 0,
                total: *size,
            });
            message.file =
                receive_file(&incoming, wire, &request.payload, name, *size, sha256).await?;
        }
    }
    if state.db.relay_receive(&scope, &id, &message)? && !state.panel_watching() {
        if message.kind == "text" {
            crate::notify::message(&message.peer_alias, &message.text);
        } else {
            crate::notify::file(
                &message.peer_alias,
                &message.peer,
                &message.name,
                message.size,
            );
        }
    }
    delivered(wire).await
}

async fn delivered(wire: &Wire) -> Result<()> {
    wire.send(&Reply::Delivered).await?;
    // Let SCTP deliver the final frame before closing the peer connection.
    let _ = tokio::time::timeout(Duration::from_secs(5), wire.recv::<bool>()).await;
    Ok(())
}

async fn checkpoint(work: &Path, file: &File, offset: u64) -> Result<()> {
    file.sync_data().await?;
    let temp = work.join("offset.tmp");
    let mut marker = File::create(&temp).await?;
    marker.write_all(&offset.to_be_bytes()).await?;
    marker.sync_all().await?;
    fs::rename(temp, work.join("offset")).await?;
    Ok(())
}

async fn receive_file(
    incoming: &IncomingTransfer<'_>,
    wire: &Wire,
    payload: &Payload,
    name: &str,
    size: u64,
    sha256: &str,
) -> Result<String> {
    let root = incoming.state.download_dir.read().await.clone();
    let (scope, id) = &incoming.key;
    let key = hex::encode(Sha256::digest(format!("{scope}/{id}")));
    let dir = PathBuf::from("AntifyBot-P2P").join(key);
    let relative = dir.join("received").join(name);
    let destination = root.join(&relative);
    let work = root.join(&dir);
    fs::create_dir_all(destination.parent().context("下载路径无效")?).await?;
    let manifest = work.join("manifest.json");
    match fs::read(&manifest).await {
        Ok(bytes) => ensure!(
            &serde_json::from_slice::<Payload>(&bytes)? == payload,
            "续传文件属性已改变"
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let temp = work.join("manifest.tmp");
            let mut file = File::create(&temp).await?;
            file.write_all(&serde_json::to_vec(payload)?).await?;
            file.sync_all().await?;
            fs::rename(temp, manifest).await?;
        }
        Err(e) => return Err(e.into()),
    }
    if destination.exists() {
        let mut file = File::open(&destination).await?;
        incoming.confirm(size);
        ensure!(
            file.metadata().await?.len() == size && hash(&mut file).await? == sha256,
            "保存位置已存在不同文件，请移动后重试"
        );
        wire.send(&Reply::Offset(size)).await?;
        return Ok(relative.to_string_lossy().into_owned());
    }
    let part = work.join("payload.part");
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&part)
        .await?;
    let mut offset = match fs::read(work.join("offset")).await {
        Ok(bytes) => u64::from_be_bytes(bytes.as_slice().try_into()?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => 0,
        Err(e) => return Err(e.into()),
    };
    ensure!(offset <= size, "本地续传进度无效");
    if offset > file.metadata().await?.len() {
        offset = 0;
        checkpoint(&work, &file, offset).await?;
    }
    ensure!(
        fs2::available_space(&root)? >= size - offset,
        "接收磁盘空间不足"
    );
    file.set_len(offset).await?;
    file.sync_data().await?;
    file.seek(std::io::SeekFrom::Start(offset)).await?;
    incoming.confirm(offset);
    wire.send(&Reply::Offset(offset)).await?;
    let mut buffer = Vec::with_capacity(WINDOW as usize);
    let mut packet = vec![0; PACKET];
    let resume_offset = offset;
    while offset < size {
        let len = (size - offset).min(WINDOW) as usize;
        let read_started = std::time::Instant::now();
        buffer.clear();
        while buffer.len() < len {
            let n = match wire.read(&mut packet).await {
                Ok(n) => n,
                Err(error) => {
                    incoming
                        .state
                        .log_event(format!(
                            "点对点收包中断，已写入 {offset} 字节，当前块收到 {} / {len} 字节",
                            buffer.len()
                        ))
                        .await;
                    return Err(error);
                }
            };
            ensure!(n <= len - buffer.len(), "收到超出声明长度的文件数据");
            buffer.extend_from_slice(&packet[..n]);
        }
        let read_time = read_started.elapsed();
        let disk_started = std::time::Instant::now();
        file.write_all(&buffer).await?;
        offset += len as u64;
        checkpoint(&work, &file, offset).await?;
        let disk_time = disk_started.elapsed();
        incoming.confirm(offset);
        wire.send(&Reply::Offset(offset)).await?;
        if offset - len as u64 == resume_offset
            || read_time >= Duration::from_secs(2)
            || disk_time >= Duration::from_secs(1)
        {
            incoming
                .state
                .log_event(format!(
                    "点对点接收 {offset} / {size} 字节，本块收包 {:.2} 秒，写盘和保存进度 {:.2} 秒",
                    read_time.as_secs_f64(),
                    disk_time.as_secs_f64()
                ))
                .await;
        }
    }
    incoming.confirm(size);
    if hash(&mut file).await? != sha256 {
        file.set_len(0).await?;
        checkpoint(&work, &file, 0).await?;
        bail!("整文件 SHA-256 校验失败，请重试");
    }
    file.sync_all().await?;
    drop(file);
    fs::hard_link(&part, &destination).await?;
    let _ = fs::remove_file(&part).await;
    Ok(relative.to_string_lossy().into_owned())
}
