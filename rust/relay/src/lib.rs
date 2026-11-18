mod direct;
mod files;
pub mod protocol;
mod store;

use axum::{
    extract::{
        ws::{Message as WsMessage, WebSocket},
        FromRequestParts, Path, State, WebSocketUpgrade,
    },
    http::{request::Parts, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use protocol::{Body, ClientEvent, SendMessage, ServerEvent};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use subtle::ConstantTimeEq;
use tokio::sync::Notify;

pub(crate) type ApiResult<T> = Result<T, ApiError>;
#[derive(Debug)]
pub struct ApiError(pub StatusCode, pub String);
impl ApiError {
    pub(crate) fn bad(message: &str) -> Self {
        Self(StatusCode::BAD_REQUEST, message.into())
    }
    pub(crate) fn internal(error: impl std::fmt::Display) -> Self {
        eprintln!("relay error: {error}");
        Self(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal server error".into(),
        )
    }
}
impl From<rusqlite::Error> for ApiError {
    fn from(e: rusqlite::Error) -> Self {
        Self::internal(e)
    }
}
impl From<std::io::Error> for ApiError {
    fn from(e: std::io::Error) -> Self {
        Self::internal(e)
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(serde_json::json!({"error":self.1}))).into_response()
    }
}
pub(crate) fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
pub fn token_hash(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}

#[derive(Clone, Deserialize)]
pub struct Device {
    pub id: String,
    pub alias: String,
    pub token_sha256: String,
}

#[derive(Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    pub listen: String,
    pub data_dir: PathBuf,
    pub devices: Vec<Device>,
    pub max_file_bytes: u64,
    pub max_storage_bytes: u64,
    pub file_ttl_secs: u64,
    pub message_ttl_secs: u64,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            listen: "127.0.0.1:53319".into(),
            data_dir: "./relay-data".into(),
            devices: vec![],
            max_file_bytes: 100 * 1024 * 1024,
            max_storage_bytes: 1024 * 1024 * 1024,
            file_ttl_secs: 86400,
            message_ttl_secs: 7 * 86400,
        }
    }
}

#[derive(Clone)]
pub struct App(Arc<Inner>);
struct Inner {
    config: Config,
    db: store::Store,
    changed: Notify,
    online: Mutex<HashMap<String, usize>>,
    direct: direct::Registry,
}
impl App {
    pub async fn open(config: Config) -> anyhow::Result<Self> {
        anyhow::ensure!(
            !config.devices.is_empty() && config.devices.len() <= 100,
            "configure 1..100 devices"
        );
        anyhow::ensure!(
            config.max_file_bytes > 0
                && config.max_storage_bytes >= config.max_file_bytes
                && config.max_storage_bytes <= i64::MAX as u64,
            "invalid file quotas"
        );
        anyhow::ensure!(
            config.file_ttl_secs >= 600
                && config.file_ttl_secs <= 365 * 86400
                && config.message_ttl_secs >= config.file_ttl_secs
                && config.message_ttl_secs <= 365 * 86400,
            "invalid retention periods"
        );
        let mut ids = std::collections::HashSet::new();
        let mut hashes = std::collections::HashSet::new();
        for d in &config.devices {
            anyhow::ensure!(
                !d.id.is_empty()
                    && d.id.len() <= 64
                    && d.id
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
                    && ids.insert(&d.id),
                "invalid or duplicate device ID"
            );
            anyhow::ensure!(
                d.token_sha256.len() == 64
                    && d.token_sha256
                        .bytes()
                        .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
                    && hashes.insert(&d.token_sha256),
                "invalid or duplicate token hash"
            );
        }
        tokio::fs::create_dir_all(config.data_dir.join("files")).await?;
        let db = store::Store::open(&config.data_dir.join("relay.db"))?;
        let app = Self(Arc::new(Inner {
            config,
            db,
            changed: Notify::new(),
            online: Mutex::new(HashMap::new()),
            direct: Default::default(),
        }));
        app.cleanup(true).await.map_err(|e| anyhow::anyhow!(e.1))?;
        Ok(app)
    }

    fn has_device(&self, id: &str) -> bool {
        self.0.config.devices.iter().any(|d| d.id == id)
    }

    async fn send(&self, from: String, mut message: SendMessage) -> ApiResult<String> {
        message.id = uuid::Uuid::parse_str(&message.id)
            .map_err(|_| ApiError::bad("id must be a UUID"))?
            .to_string();
        if message.to == from || !self.has_device(&message.to) {
            return Err(ApiError::bad("invalid recipient"));
        }
        match &message.body {
            Body::Text { text } if text.is_empty() || text.len() > 32 * 1024 => {
                return Err(ApiError::bad("text must be 1..32768 bytes"))
            }
            Body::File { file_id } if uuid::Uuid::parse_str(file_id).is_err() => {
                return Err(ApiError::bad("invalid file ID"))
            }
            _ => (),
        }
        let id = message.id.clone();
        self.0.db.send(from, message).await?;
        self.0.changed.notify_waiters();
        Ok(id)
    }

    async fn ack(&self, device: String, id: String) -> ApiResult<()> {
        self.0.db.ack(device, id).await?;
        self.0.changed.notify_waiters();
        Ok(())
    }

    pub async fn cleanup(&self, startup: bool) -> ApiResult<()> {
        for id in self.0.db.expired_files(startup).await? {
            for suffix in ["", ".part"] {
                match tokio::fs::remove_file(self.file_path(&id, suffix)).await {
                    Ok(()) => (),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                    Err(e) => return Err(e.into()),
                }
            }
            self.0.db.remove_file(id).await?;
        }
        self.0.db.prune(self.0.config.message_ttl_secs).await
    }
    fn file_path(&self, id: &str, suffix: &str) -> PathBuf {
        self.0
            .config
            .data_dir
            .join("files")
            .join(format!("{id}{suffix}"))
    }
}

pub struct Auth(String);
impl FromRequestParts<App> for Auth {
    type Rejection = ApiError;
    async fn from_request_parts(parts: &mut Parts, state: &App) -> ApiResult<Self> {
        let token = parts
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .filter(|t| t.len() >= 32 && t.len() <= 256)
            .ok_or_else(|| {
                ApiError(
                    StatusCode::UNAUTHORIZED,
                    "valid Bearer token required".into(),
                )
            })?;
        let hash = token_hash(token);
        state
            .0
            .config
            .devices
            .iter()
            .find(|d| bool::from(d.token_sha256.as_bytes().ct_eq(hash.as_bytes())))
            .map(|d| Auth(d.id.clone()))
            .ok_or_else(|| ApiError(StatusCode::UNAUTHORIZED, "invalid token".into()))
    }
}

pub fn router(app: App) -> Router {
    Router::new()
        .route("/health", get(|| async { Json(serde_json::json!({"ok":true,"service":"antify-relay","version":env!("CARGO_PKG_VERSION")})) }))
        .route("/v1/devices", get(devices))
        .route("/v1/ws", get(upgrade))
        .route("/v1/p2p", get(direct::upgrade))
        .route("/v1/messages", get(inbox).post(send_message))
        .route("/v1/messages/{id}/ack", post(ack))
        .route("/v1/files", post(files::upload))
        .route("/v1/files/{id}", get(files::download))
        .route("/v1/files/{id}/info", get(files::info))
        .layer(axum::extract::DefaultBodyLimit::max(64 * 1024))
        .with_state(app)
}

#[derive(Serialize)]
struct DeviceView {
    id: String,
    alias: String,
    online: bool,
}
async fn devices(State(app): State<App>, Auth(_): Auth) -> Json<Vec<DeviceView>> {
    let online = app.0.online.lock().unwrap_or_else(|e| e.into_inner());
    Json(
        app.0
            .config
            .devices
            .iter()
            .map(|d| DeviceView {
                id: d.id.clone(),
                alias: d.alias.clone(),
                online: online.get(&d.id).copied().unwrap_or(0) > 0,
            })
            .collect(),
    )
}
async fn send_message(
    State(app): State<App>,
    Auth(from): Auth,
    Json(message): Json<SendMessage>,
) -> ApiResult<Json<ServerEvent>> {
    Ok(Json(ServerEvent::Accepted {
        id: app.send(from, message).await?,
    }))
}
async fn inbox(State(app): State<App>, Auth(device): Auth) -> ApiResult<Json<Vec<ServerEvent>>> {
    Ok(Json(app.0.db.events(device).await?))
}
async fn ack(
    State(app): State<App>,
    Auth(device): Auth,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    app.ack(device, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

// RAII keeps the online count correct even if the connection task is cancelled.
struct ConnectionGuard(App, String);
impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        let mut online = self.0 .0.online.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(n) = online.get_mut(&self.1) {
            *n = n.saturating_sub(1);
        }
    }
}
async fn upgrade(
    State(app): State<App>,
    Auth(device): Auth,
    ws: WebSocketUpgrade,
) -> ApiResult<Response> {
    {
        let mut online = app.0.online.lock().unwrap_or_else(|e| e.into_inner());
        let n = online.entry(device.clone()).or_default();
        if *n >= 2 {
            return Err(ApiError(
                StatusCode::TOO_MANY_REQUESTS,
                "maximum 2 connections per device".into(),
            ));
        }
        *n += 1;
    }
    let guard = ConnectionGuard(app.clone(), device.clone());
    Ok(ws
        .max_message_size(64 * 1024)
        .max_frame_size(64 * 1024)
        .on_upgrade(move |socket| session(socket, app, device, guard)))
}
async fn emit(socket: &mut WebSocket, event: &ServerEvent) -> bool {
    let Ok(json) = serde_json::to_string(event) else {
        return false;
    };
    matches!(
        tokio::time::timeout(
            Duration::from_secs(10),
            socket.send(WsMessage::Text(json.into()))
        )
        .await,
        Ok(Ok(()))
    )
}
async fn push_pending(socket: &mut WebSocket, app: &App, device: &str) -> bool {
    match app.0.db.events(device.to_string()).await {
        Ok(events) => {
            for event in events {
                if !emit(socket, &event).await {
                    return false;
                }
            }
            true
        }
        Err(_) => false,
    }
}
async fn session(mut socket: WebSocket, app: App, device: String, _guard: ConnectionGuard) {
    if !emit(
        &mut socket,
        &ServerEvent::Ready {
            device_id: device.clone(),
        },
    )
    .await
    {
        return;
    }
    let mut retry = tokio::time::interval(Duration::from_secs(5));
    let mut heartbeat = tokio::time::interval(Duration::from_secs(20));
    let mut last_seen = tokio::time::Instant::now();
    loop {
        tokio::select! {
            incoming = socket.recv() => {
                let Some(Ok(message)) = incoming else { break; };
                last_seen = tokio::time::Instant::now();
                let result = match message {
                    WsMessage::Text(text) => match serde_json::from_str::<ClientEvent>(&text) {
                        Ok(ClientEvent::Send { message }) => app.send(device.clone(), message).await.map(|id| Some(ServerEvent::Accepted { id })),
                        Ok(ClientEvent::Ack { id }) => app.ack(device.clone(), id).await.map(|_| None),
                        Err(_) => Err(ApiError::bad("invalid event JSON")),
                    },
                    WsMessage::Close(_) => break,
                    WsMessage::Ping(_) | WsMessage::Pong(_) => Ok(None),
                    _ => Err(ApiError::bad("text JSON frames only")),
                };
                let response = match result {
                    Ok(event) => event,
                    Err(e) => Some(ServerEvent::Error { code:e.0.as_u16(), message:e.1 }),
                };
                if let Some(event) = response { if !emit(&mut socket, &event).await { break; } }
            }
            _ = retry.tick() => { if !push_pending(&mut socket, &app, &device).await { break; } }
            _ = app.0.changed.notified() => { if !push_pending(&mut socket, &app, &device).await { break; } }
            _ = heartbeat.tick() => {
                if last_seen.elapsed() > Duration::from_secs(60) { break; }
                if !matches!(tokio::time::timeout(Duration::from_secs(10), socket.send(WsMessage::Ping(Vec::new().into()))).await, Ok(Ok(()))) { break; }
            }
        }
    }
}
