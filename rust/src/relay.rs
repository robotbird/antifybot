//! Remote relay transport. Credentials stay in Rust; the panel only sees status.
mod files;

use crate::{
    db::RelayPending,
    state::{now_ms, ChatMsg, Shared},
};
use antify_protocol::{Body, FileInfo, SendMessage, ServerEvent};
use anyhow::{bail, ensure, Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{atomic::Ordering, Mutex},
    time::Duration,
};
use tokio::sync::{Notify, RwLock};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, Message as Frame},
};

#[derive(Clone, Default, Deserialize, Serialize)]
pub struct Settings {
    pub enabled: bool,
    pub server_url: String,
    pub token: String,
}
impl Settings {
    pub fn scope(&self) -> String {
        hex::encode(Sha256::digest(
            format!("{}\n{}", self.server_url, self.token).as_bytes(),
        ))[..20]
            .to_string()
    }
    fn validate(&mut self) -> Result<()> {
        self.server_url = self.server_url.trim().trim_end_matches('/').to_string();
        if !self.enabled && self.server_url.is_empty() {
            return Ok(());
        }
        let url = reqwest::Url::parse(&self.server_url).context("服务器地址格式不正确")?;
        ensure!(
            url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none(),
            "地址中不能包含账户、查询参数或片段"
        );
        let loopback = url.host_str().is_some_and(|h| {
            h == "localhost"
                || h.parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback())
        });
        ensure!(
            url.scheme() == "https" || (url.scheme() == "http" && loopback),
            "使用 HTTPS 地址；SSH 隧道可使用 http://127.0.0.1:53320"
        );
        if self.enabled {
            ensure!(
                self.token.len() >= 32
                    && self.token.len() <= 256
                    && self.token.bytes().all(|b| b.is_ascii_graphic()),
                "请填写有效的设备令牌"
            );
        }
        Ok(())
    }
}

#[derive(Clone, Default, Serialize)]
pub struct Status {
    pub connected: bool,
    pub device_id: String,
    pub error: String,
}
#[derive(Clone, Deserialize)]
pub struct Peer {
    pub id: String,
    pub alias: String,
    pub online: bool,
}

pub struct Runtime {
    settings: RwLock<Settings>,
    status: RwLock<Status>,
    peers: RwLock<Vec<Peer>>,
    changed: Notify,
}
impl Runtime {
    pub fn load(dir: &Path) -> Self {
        let mut cfg: Settings = std::fs::read(dir.join("relay.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let error = match cfg.validate() {
            Ok(()) => String::new(),
            Err(e) => {
                cfg.enabled = false;
                e.to_string()
            }
        };
        Self {
            settings: RwLock::new(cfg),
            status: RwLock::new(Status {
                error,
                ..Default::default()
            }),
            peers: RwLock::new(vec![]),
            changed: Notify::new(),
        }
    }
}

pub fn is_remote(peer: &str) -> bool {
    peer.starts_with("relay:")
}
fn peer_key(cfg: &Settings, id: &str) -> String {
    format!("relay:{}:{id}", cfg.scope())
}
fn recipient<'a>(cfg: &Settings, peer: &'a str) -> Result<&'a str> {
    ensure!(cfg.enabled, "请先在设置中启用远程连接");
    peer.strip_prefix(&format!("relay:{}:", cfg.scope()))
        .filter(|s| !s.is_empty())
        .context("该会话不属于当前中转配置，请重新选择设备")
}

pub async fn view(state: &Shared) -> serde_json::Value {
    let cfg = state.relay.settings.read().await;
    let status = state.relay.status.read().await;
    serde_json::json!({"enabled":cfg.enabled,"server_url":cfg.server_url,"has_token":!cfg.token.is_empty(),"connected":status.connected,"device_id":status.device_id,"error":status.error})
}
pub async fn devices(state: &Shared) -> Vec<serde_json::Value> {
    let cfg = state.relay.settings.read().await;
    let status = state.relay.status.read().await;
    if !cfg.enabled {
        return vec![];
    }
    state.relay.peers.read().await.iter().filter(|p| p.id != status.device_id).map(|p| serde_json::json!({
        "fingerprint":peer_key(&cfg,&p.id),"alias":p.alias,"online":status.connected && p.online,
        "transport":"relay","deviceType":"desktop","ip":"","port":null,"lastSeenMs":0,"version":"relay-v1",
    })).collect()
}

/// Blank token means retain the previously saved credential; never echo it back.
pub async fn configure(state: &Shared, mut cfg: Settings) -> Result<()> {
    if cfg.token.is_empty() {
        cfg.token = state.relay.settings.read().await.token.clone();
    }
    cfg.validate()?;
    let path = state.identity.cfg_dir.join("relay.json");
    let temp = state
        .identity
        .cfg_dir
        .join(format!(".relay-{}.tmp", uuid::Uuid::new_v4()));
    let mut options = std::fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    {
        use std::io::Write;
        let mut file = options.open(&temp)?;
        file.write_all(&serde_json::to_vec_pretty(&cfg)?)?;
        file.sync_all()?;
    }
    std::fs::rename(&temp, &path)?;
    let changed_scope = cfg.scope() != state.relay.settings.read().await.scope();
    *state.relay.settings.write().await = cfg;
    *state.relay.status.write().await = Status::default();
    if changed_scope {
        state.relay.peers.write().await.clear();
    }
    state.relay.changed.notify_one();
    Ok(())
}

async fn target(state: &Shared, peer: &str) -> Result<(Settings, String, String)> {
    let cfg = state.relay.settings.read().await.clone();
    let id = recipient(&cfg, peer)?.to_string();
    let peers = state.relay.peers.read().await;
    let alias = peers
        .iter()
        .find(|p| p.id == id)
        .map(|p| p.alias.clone())
        .context("远程设备不存在，请等待连接恢复")?;
    Ok((cfg, id, alias))
}
pub async fn enqueue_text(state: &Shared, peer: &str, text: String) -> Result<u64> {
    ensure!(
        !text.is_empty() && text.len() <= 32 * 1024,
        "文字最多 32 KiB"
    );
    let (cfg, to, alias) = target(state, peer).await?;
    let body = serde_json::to_string(&Body::Text { text: text.clone() })?;
    let id = state.db.relay_enqueue(
        &cfg.scope(),
        &uuid::Uuid::new_v4().to_string(),
        &to,
        Some(&body),
        &ChatMsg {
            out: true,
            peer: peer.into(),
            peer_alias: alias,
            kind: "text".into(),
            text,
            at: now_ms(),
            status: "queued".into(),
            ..Default::default()
        },
    )?;
    Ok(id)
}
pub async fn enqueue_file(
    state: &Shared,
    peer: &str,
    path: PathBuf,
    name: Option<String>,
) -> Result<u64> {
    let (cfg, to, alias) = target(state, peer).await?;
    let meta = tokio::fs::metadata(&path).await?;
    ensure!(
        meta.is_file() && meta.len() <= files::MAX_FILE,
        "中转文件须为不超过 100 MiB 的普通文件"
    );
    let name = name.unwrap_or_else(|| {
        path.file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
    });
    files::safe_name(&name)?;
    let id = state.db.relay_enqueue(
        &cfg.scope(),
        &uuid::Uuid::new_v4().to_string(),
        &to,
        None,
        &ChatMsg {
            out: true,
            peer: peer.into(),
            peer_alias: alias,
            kind: "file".into(),
            name: name.clone(),
            size: meta.len(),
            at: now_ms(),
            status: "queued".into(),
            src_path: path.to_string_lossy().into_owned(),
            ..Default::default()
        },
    )?;
    if meta.len() <= 8 * 1024 * 1024 {
        let mime = files::image_mime(&name);
        if let Some(mime) = mime {
            if let Ok(bytes) = tokio::fs::read(&path).await {
                state.cache_image(id, mime.into(), bytes).await;
            }
        }
    }
    Ok(id)
}
pub async fn enqueue_stream(
    state: &Shared,
    peer: &str,
    name: String,
    size: u64,
    body: axum::body::Body,
) -> Result<u64> {
    target(state, peer).await?;
    files::safe_name(&name)?;
    ensure!(size <= files::MAX_FILE, "单文件最多 100 MiB");
    let dir = state.identity.cfg_dir.join("relay-outbox");
    tokio::fs::create_dir_all(&dir).await?;
    let path = dir.join(uuid::Uuid::new_v4().to_string());
    let result = files::stage(&path, size, body).await;
    if let Err(e) = result {
        let _ = tokio::fs::remove_file(&path).await;
        return Err(e);
    }
    match enqueue_file(state, peer, path.clone(), Some(name)).await {
        Ok(id) => Ok(id),
        Err(e) => {
            let _ = tokio::fs::remove_file(path).await;
            Err(e)
        }
    }
}
pub async fn retry(state: &Shared, id: u64) -> Result<()> {
    let cfg = state.relay.settings.read().await;
    ensure!(cfg.enabled, "请启用远程连接");
    state.db.relay_retry(&cfg.scope(), id)
}

pub fn start(state: Shared) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut backoff = 1u64;
        loop {
            let changed = state.relay.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            let cfg = state.relay.settings.read().await.clone();
            if !cfg.enabled {
                changed.await;
                continue;
            }
            tokio::select! {
                _ = &mut changed => { backoff=1; }
                result = session(&state,&cfg) => {
                    let error = result.err().map(|e| e.to_string()).unwrap_or_else(|| "连接已关闭".into());
                    { let mut s = state.relay.status.write().await; s.connected=false; s.error=error; }
                    tokio::select! { _ = &mut changed => { backoff=1; }, _ = tokio::time::sleep(Duration::from_secs(backoff)) => { backoff=(backoff*2).min(15); } }
                }
            }
        }
    })
}

fn http() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .use_rustls_tls()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(15))
        .build()?)
}
async fn ack(client: &reqwest::Client, cfg: &Settings, id: &str) -> Result<()> {
    client
        .post(format!("{}/v1/messages/{id}/ack", cfg.server_url))
        .bearer_auth(&cfg.token)
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

async fn session(state: &Shared, cfg: &Settings) -> Result<()> {
    let client = http()?;
    let url = format!("{}/v1/ws", cfg.server_url.replacen("http", "ws", 1));
    let mut request = url.into_client_request()?;
    request
        .headers_mut()
        .insert("authorization", format!("Bearer {}", cfg.token).parse()?);
    let (mut socket, _) =
        tokio::time::timeout(Duration::from_secs(10), connect_async(request)).await??;
    let frame = tokio::time::timeout(Duration::from_secs(10), socket.next())
        .await?
        .context("服务端未返回设备身份")??;
    let ServerEvent::Ready { device_id } = serde_json::from_str(frame.to_text()?)? else {
        bail!("中转服务握手异常");
    };
    *state.relay.status.write().await = Status {
        connected: true,
        device_id: device_id.clone(),
        error: String::new(),
    };
    let (mut writer, mut reader) = socket.split();
    let (tx, mut rx) = tokio::sync::mpsc::channel::<ServerEvent>(256);
    let inflight = Mutex::new(HashSet::<String>::new());
    let read = async {
        loop {
            let frame = tokio::time::timeout(Duration::from_secs(65), reader.next())
                .await?
                .context("中转连接已关闭")??;
            match frame {
                Frame::Ping(data) => {
                    tokio::time::timeout(Duration::from_secs(10), writer.send(Frame::Pong(data)))
                        .await??;
                }
                Frame::Close(_) => bail!("中转连接已关闭"),
                Frame::Text(text) => {
                    let event: ServerEvent = serde_json::from_str(&text)?;
                    let key = event_key(&event);
                    if let Some(key) = key {
                        let mut set = inflight.lock().unwrap();
                        if set.insert(key) {
                            tx.try_send(event)
                                .context("接收队列已满，正在重新连接补收")?;
                        }
                    }
                }
                _ => (),
            }
        }
        #[allow(unreachable_code)]
        Ok::<(), anyhow::Error>(())
    };
    let process = async {
        while let Some(event) = rx.recv().await {
            let key = event_key(&event).unwrap();
            if let Err(e) = receive(state, cfg, &client, &device_id, event).await {
                state.relay.status.write().await.error = format!("接收失败，将重试：{e}");
            }
            inflight.lock().unwrap().remove(&key);
        }
        Ok::<(), anyhow::Error>(())
    };
    let peers = async {
        let mut tick = tokio::time::interval(Duration::from_secs(5));
        loop {
            tick.tick().await;
            let peers = client
                .get(format!("{}/v1/devices", cfg.server_url))
                .bearer_auth(&cfg.token)
                .send()
                .await?
                .error_for_status()?
                .json::<Vec<Peer>>()
                .await?;
            *state.relay.peers.write().await = peers;
        }
        #[allow(unreachable_code)]
        Ok::<(), anyhow::Error>(())
    };
    tokio::select! { r=read=>r, r=process=>r, r=peers=>r, r=outbox(state,cfg,&client)=>r }
}
fn event_key(event: &ServerEvent) -> Option<String> {
    match event {
        ServerEvent::Message { message } => Some(format!("m:{}", message.id)),
        ServerEvent::Delivered { id } => Some(format!("d:{id}")),
        _ => None,
    }
}

async fn receive(
    state: &Shared,
    cfg: &Settings,
    client: &reqwest::Client,
    me: &str,
    event: ServerEvent,
) -> Result<()> {
    let scope = cfg.scope();
    match event {
        ServerEvent::Delivered { id } => {
            if let Some((local, path)) = state.db.relay_delivered(&scope, &id)? {
                state.set_msg_status(local, "ok").await;
                if Path::new(&path).starts_with(state.identity.cfg_dir.join("relay-outbox")) {
                    let _ = tokio::fs::remove_file(path).await;
                }
            }
            ack(client, cfg, &id).await?;
        }
        ServerEvent::Message { message } => {
            uuid::Uuid::parse_str(&message.id)?;
            ensure!(message.to == me, "收到非本机消息");
            if !state.db.relay_seen(&scope, &message.id)? {
                let alias = state
                    .relay
                    .peers
                    .read()
                    .await
                    .iter()
                    .find(|p| p.id == message.from)
                    .map(|p| p.alias.clone())
                    .unwrap_or_else(|| message.from.clone());
                let mut msg = ChatMsg {
                    peer: peer_key(cfg, &message.from),
                    peer_alias: alias,
                    at: (message.created_at.max(0) as u64).saturating_mul(1000),
                    ..Default::default()
                };
                match message.body {
                    Body::Text { text } => {
                        msg.kind = "text".into();
                        msg.text = text;
                    }
                    Body::File { file_id } => {
                        let info: FileInfo = client
                            .get(format!("{}/v1/files/{file_id}/info", cfg.server_url))
                            .bearer_auth(&cfg.token)
                            .send()
                            .await?
                            .error_for_status()?
                            .json()
                            .await?;
                        let file = files::download(state, cfg, client, &message.id, &info).await?;
                        msg.kind = "file".into();
                        msg.name = info.name;
                        msg.size = info.size;
                        msg.file = file;
                    }
                }
                if state.db.relay_receive(&scope, &message.id, &msg)? && !state.panel_watching() {
                    if msg.kind == "text" {
                        crate::notify::message(&msg.peer_alias, &msg.text);
                    } else {
                        crate::notify::file(&msg.peer_alias, &msg.peer, &msg.name, msg.size);
                    }
                }
            }
            ack(client, cfg, &message.id).await?;
        }
        _ => (),
    }
    Ok(())
}

async fn outbox(state: &Shared, cfg: &Settings, client: &reqwest::Client) -> Result<()> {
    let mut tick = tokio::time::interval(Duration::from_secs(2));
    loop {
        tick.tick().await;
        for item in state.db.relay_pending(&cfg.scope())? {
            if let Err(e) = send_pending(state, cfg, client, &item).await {
                let status = e.downcast_ref::<reqwest::Error>().and_then(|e| e.status());
                let permanent = e.downcast_ref::<std::io::Error>().is_some()
                    || status.is_some_and(|s| {
                        s.is_client_error() && s.as_u16() != 408 && s.as_u16() != 429
                    })
                    || (!item.src_path.is_empty() && state.send_cancel.load(Ordering::Relaxed));
                state.db.relay_status(
                    &cfg.scope(),
                    &item.remote_id,
                    if permanent { "fail" } else { "queued" },
                )?;
                state.relay.status.write().await.error = format!(
                    "发送失败{}：{e}",
                    if permanent {
                        "，请重试"
                    } else {
                        "，稍后自动重试"
                    }
                );
            }
        }
    }
}
async fn send_pending(
    state: &Shared,
    cfg: &Settings,
    client: &reqwest::Client,
    item: &RelayPending,
) -> Result<()> {
    let body = match &item.body {
        Some(body) => serde_json::from_str::<Body>(body)?,
        None => {
            state
                .db
                .relay_status(&cfg.scope(), &item.remote_id, "sending")?;
            let info = files::upload(state, cfg, client, item).await?;
            let body = Body::File { file_id: info.id };
            state.db.relay_body(
                &cfg.scope(),
                &item.remote_id,
                &serde_json::to_string(&body)?,
            )?;
            body
        }
    };
    let reply: ServerEvent = client
        .post(format!("{}/v1/messages", cfg.server_url))
        .bearer_auth(&cfg.token)
        .json(&SendMessage {
            id: item.remote_id.clone(),
            to: item.recipient.clone(),
            body,
        })
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(
        matches!(reply,ServerEvent::Accepted { ref id } if *id == item.remote_id),
        "服务端未确认消息"
    );
    state
        .db
        .relay_status(&cfg.scope(), &item.remote_id, "accepted")?;
    Ok(())
}
