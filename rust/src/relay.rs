//! Direct transport and signaling. Credentials stay in Rust; the panel only sees status.
mod direct;
mod files;

use crate::{
    db::RelayPending,
    state::{now_ms, ChatMsg, Shared},
};
use antify_protocol::DirectDevice;
use anyhow::{bail, ensure, Context, Result};
use direct::transfer::{Payload, MAX_FILE};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{atomic::Ordering, Mutex},
    time::Duration,
};
use tokio::sync::{Notify, RwLock};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, Message as Frame},
};

const DEFAULT_SIGNAL_URL: &str = "https://p2p.bjjxysbz.com";
const LEGACY_SIGNAL_URL: &str = "http://127.0.0.1:53320";

#[derive(Clone, Default, Deserialize, Serialize)]
pub struct Settings {
    pub enabled: bool,
    pub server_url: String,
    pub token: String,
    #[serde(default)]
    pub prefer_p2p: bool,
}
impl Settings {
    pub fn scope(&self) -> String {
        // Both addresses reach the same deployment and share its saved transfers.
        let storage_url = if self.server_url == DEFAULT_SIGNAL_URL {
            LEGACY_SIGNAL_URL
        } else {
            &self.server_url
        };
        hex::encode(Sha256::digest(
            format!("{}\n{}", storage_url, self.token).as_bytes(),
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

#[derive(Clone, Serialize)]
pub struct ReceiveProgress {
    pub peer: String,
    pub sender: String,
    pub name: String,
    pub file_type: String,
    pub verifying: bool,
    pub relayed: bool,
    pub got: u64,
    pub total: u64,
}

pub struct Runtime {
    settings: RwLock<Settings>,
    status: RwLock<Status>,
    online: RwLock<HashSet<String>>,
    changed: Notify,
    pub progress: Mutex<crate::state::SendProgress>,
    receiving: Mutex<HashMap<(String, String), Option<ReceiveProgress>>>,
}
impl Runtime {
    pub fn receiving_files(&self) -> Vec<ReceiveProgress> {
        self.receiving.lock().unwrap().values().flatten().cloned().collect()
    }

    pub fn load(dir: &Path) -> Self {
        let mut cfg: Settings = std::fs::read(dir.join("relay.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        if matches!(cfg.server_url.trim().trim_end_matches('/'), "" | LEGACY_SIGNAL_URL) {
            cfg.server_url = DEFAULT_SIGNAL_URL.into();
        }
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
            online: RwLock::new(HashSet::new()),
            changed: Notify::new(),
            progress: Mutex::new(Default::default()),
            receiving: Mutex::new(HashMap::new()),
        }
    }
}

pub fn is_remote(peer: &str) -> bool {
    peer.starts_with("relay:")
}
fn peer_key(cfg: &Settings, id: &str) -> String {
    format!("relay:{}:{id}", cfg.scope())
}
pub async fn route_remote(state: &Shared, peer: &str) -> bool {
    if is_remote(peer) {
        return true;
    }
    if state
        .devices
        .lock()
        .await
        .get(peer)
        .is_some_and(|d| now_ms().saturating_sub(d.last_seen) < 10_000)
    {
        return false;
    }
    let cfg = state.relay.settings.read().await;
    cfg.enabled
        && state
            .db
            .direct_devices(&cfg.scope())
            .unwrap_or_default()
            .iter()
            .any(|device| device.fingerprint == peer)
}

pub async fn view(state: &Shared) -> serde_json::Value {
    let cfg = state.relay.settings.read().await;
    let status = state.relay.status.read().await;
    serde_json::json!({"enabled":cfg.enabled,"server_url":cfg.server_url,"prefer_p2p":cfg.prefer_p2p,"has_token":!cfg.token.is_empty(),"connected":status.connected,"device_id":status.device_id,"error":status.error})
}
pub async fn devices(state: &Shared) -> Vec<serde_json::Value> {
    let cfg = state.relay.settings.read().await;
    let status = state.relay.status.read().await;
    if !cfg.enabled {
        return vec![];
    }
    let online = state.relay.online.read().await;
    state.db.direct_devices(&cfg.scope()).unwrap_or_default().into_iter()
        .filter(|p| p.id != status.device_id && p.fingerprint != state.identity.fingerprint)
        .map(|p| serde_json::json!({
            "fingerprint":p.fingerprint,"legacyPeer":peer_key(&cfg,&p.id),"alias":p.alias,
            "online":status.connected && online.contains(&p.id),"p2p":true,
            "transport":"p2p","deviceType":"desktop","ip":"","port":null,"lastSeenMs":0,"version":"p2p-v1",
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
        state.relay.online.write().await.clear();
    }
    state.relay.changed.notify_one();
    Ok(())
}

async fn target(state: &Shared, peer: &str) -> Result<(Settings, String, String)> {
    let cfg = state.relay.settings.read().await.clone();
    ensure!(cfg.enabled, "请先在设置中启用远程连接");
    let device = state
        .db
        .direct_devices(&cfg.scope())?
        .into_iter()
        .find(|p| p.fingerprint == peer || peer_key(&cfg, &p.id) == peer)
        .context("远程设备尚未完成身份关联，请等待连接")?;
    Ok((cfg, device.id, device.alias))
}
pub async fn enqueue_text(state: &Shared, peer: &str, text: String) -> Result<u64> {
    ensure!(
        !text.is_empty() && text.len() <= 32 * 1024,
        "文字最多 32 KiB"
    );
    let (cfg, to, alias) = target(state, peer).await?;
    let body = serde_json::to_string(&Payload::Text { text: text.clone() })?;
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
        meta.is_file() && meta.len() <= MAX_FILE,
        "直传文件须为不超过 16 GiB 的普通文件"
    );
    let name = name.unwrap_or_else(|| {
        path.file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
    });
    files::safe_name(&name)?;
    let mut source = tokio::fs::File::open(&path).await?;
    let sha256 = direct::transfer::hash(&mut source).await?;
    ensure!(
        source.metadata().await?.len() == meta.len(),
        "源文件在读取时发生变化"
    );
    let payload = serde_json::to_string(&Payload::File {
        name: name.clone(),
        size: meta.len(),
        sha256,
    })?;
    let id = state.db.relay_enqueue(
        &cfg.scope(),
        &uuid::Uuid::new_v4().to_string(),
        &to,
        Some(&payload),
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
    ensure!(size <= MAX_FILE, "单文件最多 16 GiB");
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
                result = direct::session(&state,&cfg) => {
                    let error = result.err().map(|e| e.to_string()).unwrap_or_else(|| "连接已关闭".into());
                    { let mut s = state.relay.status.write().await; s.connected=false; s.error=error; }
                    tokio::select! { _ = &mut changed => { backoff=1; }, _ = tokio::time::sleep(Duration::from_secs(backoff)) => { backoff=(backoff*2).min(15); } }
                }
            }
        }
    })
}
