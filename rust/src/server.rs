//! HTTPS 服务器：LocalSend v2 API（官方 App 可直接发文件给我们）+ 面板 API
use crate::state::{now_ms, Device, IncomingFile, ReceivedFile, Session, Shared};
use axum::extract::{ConnectInfo, Query, Request, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Json, Response};
use axum::routing::{get, post};
use axum::Router;
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;

/// 协议 router：LocalSend v2 端点（官方 App 兼容）。
/// 绑 0.0.0.0 对局域网开放 —— 绝不能混入面板 API（任意路径发送/移除设备等只许本机调用）。
pub fn build_protocol_router(state: Shared) -> Router {
    Router::new()
        .route("/api/localsend/v2/info", get(info))
        .route("/api/localsend/v1/info", get(info))
        .route("/api/localsend/v2/register", post(register))
        .route("/api/localsend/v2/prepare-upload", post(prepare_upload))
        .route("/api/localsend/v2/upload", post(upload))
        .route("/api/localsend/v2/cancel", post(cancel))
        .route("/api/localsend/v2/cancel-upload", post(cancel)) // 旧草案别名
        .with_state(state)
}

/// 面板 router：仪表盘页面 + 面板 API（仅绑 127.0.0.1）
pub fn build_panel_router(state: Shared) -> Router {
    Router::new()
        .route("/", get(dashboard))
        .route("/api/ui/state", get(ui_state))
        .route("/api/ui/send", post(ui_send))
        .route("/api/ui/send-text", post(ui_send_text))
        .route("/api/ui/add", post(ui_add))
        .route("/api/ui/remove-device", post(ui_remove_device))
        .route("/api/ui/pick", post(ui_pick))
        .route("/api/ui/send-path", post(ui_send_path))
        .route("/api/ui/retry", post(ui_retry))
        .route("/api/ui/reveal", post(ui_reveal))
        .route("/api/ui/asset", get(ui_asset))
        .route("/api/ui/set-dir", post(ui_set_dir))
        .route("/api/ui/check-update", post(ui_check_update))
        .route("/api/ui/open-url", post(ui_open_url))
        .with_state(state)
}

/// 面板的明文 HTTP 版（仅 127.0.0.1）：给 Tauri WebView 与 CLI `serve` 用
pub async fn serve_ui_http(state: Shared, port: u16) -> anyhow::Result<()> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let app = build_panel_router(state).into_make_service_with_connect_info::<SocketAddr>();
    axum_server::bind(addr)
        .serve(app)
        .await
        .map_err(|e| anyhow::anyhow!("UI HTTP 服务退出: {e}"))
}

/// 从 preferred 起找一个能绑定的 TCP 端口（0.0.0.0 实测绑定后立即释放）。
/// Windows 上同机跑官方 LocalSend 时 53317 会被占用：协议允许任意端口
/// （多播公告 / register 载荷携带真实端口），顺延即可互通。
pub fn pick_free_port(preferred: u16) -> u16 {
    let mut port = preferred;
    for _ in 0..32 {
        if std::net::TcpListener::bind(("0.0.0.0", port)).is_ok() {
            return port;
        }
        port = port.saturating_add(1);
    }
    // 连续 32 个都被占：交给操作系统随机分配
    std::net::TcpListener::bind(("0.0.0.0", 0))
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .unwrap_or(preferred)
}

/// axum-server + 自签 rustls（LocalSend 语义：客户端不校验证书）
pub async fn serve(state: Shared) -> anyhow::Result<()> {
    let tls = axum_server::tls_rustls::RustlsConfig::from_pem_file(
        state.identity.cert_pem.clone(),
        state.identity.key_pem.clone(),
    )
    .await?;
    let addr = SocketAddr::from(([0, 0, 0, 0], state.identity.port));
    let app = build_protocol_router(state.clone()).into_make_service_with_connect_info::<SocketAddr>();
    // 会话清扫：发送方崩溃/不取消时，60 秒无活动自动回收，避免锁死后续接收
    {
        let st = state.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                let slot = st.session.lock().await;
                if let Some(s) = slot.as_ref() {
                    if s.last_active.elapsed() > std::time::Duration::from_secs(60) {
                        let alias = s.sender_alias.clone();
                        drop(slot);
                        *st.session.lock().await = None;
                        st.log_event(format!("「{alias}」的会话 60 秒无活动，自动回收")).await;
                    }
                }
            }
        });
    }

    axum_server::bind_rustls(addr, tls)
        .serve(app)
        .await
        .map_err(|e| anyhow::anyhow!("HTTPS 服务退出: {e}"))
}

fn err_json(status: StatusCode, msg: &str) -> Response {
    (status, Json(json!({ "error": msg }))).into_response()
}

// ═══════════════ v2 协议端点 ═══════════════

async fn info(State(state): State<Shared>) -> Response {
    let id = &state.identity;
    Json(json!({
        "alias": id.alias,
        "version": crate::config::PROTOCOL_VERSION,
        "deviceModel": id.device_model,
        "deviceType": "headless",
        "fingerprint": id.fingerprint,
        "download": false,
    }))
    .into_response()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")] // LocalSend 载荷是 camelCase（deviceModel/deviceType）
struct RegisterIn {
    #[serde(default)]
    #[allow(dead_code)]
    alias: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    device_model: Option<String>,
    #[serde(default)]
    device_type: Option<String>,
    #[serde(default)]
    fingerprint: String,
    #[serde(default = "default_port")]
    port: u16,
    #[serde(default = "default_https")]
    protocol: String,
    #[serde(default)]
    download: bool,
}
fn default_port() -> u16 {
    53317
}
fn default_https() -> String {
    "https".to_string()
}

async fn register(
    State(state): State<Shared>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> Response {
    // 宽松解析：容忍字段缺失的异构实现
    let Ok(reg) = serde_json::from_value::<RegisterIn>(body.clone()) else {
        return err_json(StatusCode::BAD_REQUEST, "register 请求体无法解析");
    };
    if !reg.fingerprint.is_empty() {
        let device = Device {
            fingerprint: reg.fingerprint.clone(),
            alias: reg.alias.clone(),
            ip: peer.ip(),
            port: reg.port,
            https: reg.protocol.eq_ignore_ascii_case("https"),
            device_model: reg.device_model,
            device_type: reg.device_type,
            version: reg.version,
            download: reg.download,
            last_seen: now_ms(),
        };
        state
            .log_event(format!("「{}」注册进来（{}）", device.alias, peer.ip()))
            .await;
        state.upsert_device(device).await;
    }
    let id = &state.identity;
    Json(json!({
        "alias": id.alias,
        "version": crate::config::PROTOCOL_VERSION,
        "deviceModel": id.device_model,
        "deviceType": "headless",
        "fingerprint": id.fingerprint,
        "download": false,
    }))
    .into_response()
}

async fn prepare_upload(
    State(state): State<Shared>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> Response {
    let info = body.get("info").cloned().unwrap_or(json!({}));
    let sender_alias = info
        .get("alias")
        .and_then(|v| v.as_str())
        .unwrap_or("未知设备")
        .to_string();
    let sender_fp = info
        .get("fingerprint")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let Some(files_raw) = body.get("files").and_then(|v| v.as_object()) else {
        return err_json(StatusCode::BAD_REQUEST, "缺少 files");
    };
    if files_raw.is_empty() {
        return err_json(StatusCode::BAD_REQUEST, "没有文件");
    }

    let mut files = HashMap::new();
    for (id, f) in files_raw {
        let name = f.get("fileName").and_then(|v| v.as_str()).unwrap_or("file");
        files.insert(
            id.clone(),
            IncomingFile {
                token: uuid::Uuid::new_v4().to_string(),
                file_name: name.to_string(),
                size: f.get("size").and_then(|v| v.as_u64()).unwrap_or(0),
                sha256: f
                    .get("sha256")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(str::to_string),
                mime: f
                    .get("fileType")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                preview: f
                    .get("preview")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                done: false,
                attempts: 0,
            },
        );
    }

    let session_id = uuid::Uuid::new_v4().to_string();
    {
        let mut slot = state.session.lock().await;
        if slot.is_some() {
            return err_json(StatusCode::CONFLICT, "Blocked by another session");
        }
        *slot = Some(Session {
            id: session_id.clone(),
            sender_ip: peer.ip(),
            sender_alias: sender_alias.clone(),
            sender_fp,
            files,
            last_active: tokio::time::Instant::now(),
        });
    }
    state
        .log_event(format!(
            "「{}」请求发送 {} 个文件，已自动接受",
            sender_alias,
            files_raw.len()
        ))
        .await;

    let tokens: serde_json::Map<String, serde_json::Value> = state
        .session
        .lock()
        .await
        .as_ref()
        .map(|s| {
            s.files
                .iter()
                .map(|(id, f)| (id.clone(), json!(f.token)))
                .collect()
        })
        .unwrap_or_default();

    Json(json!({
        "sessionId": session_id,
        "files": tokens,
    }))
    .into_response()
}

#[derive(Deserialize)]
struct UploadQuery {
    #[serde(rename = "sessionId")]
    session_id: String,
    #[serde(rename = "fileId")]
    file_id: String,
    token: String,
}

async fn upload(
    State(state): State<Shared>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Query(q): Query<UploadQuery>,
    _headers: HeaderMap,
    req: Request,
) -> Response {
    // 会话与令牌校验
    let (file_name, size, sha_expect, mime, preview) = {
        let mut slot = state.session.lock().await;
        let Some(session) = slot.as_mut() else {
            return err_json(StatusCode::FORBIDDEN, "Invalid token or IP address");
        };
        if session.id != q.session_id || session.sender_ip != peer.ip() {
            return err_json(StatusCode::FORBIDDEN, "Invalid token or IP address");
        }
        let Some(file) = session.files.get_mut(&q.file_id) else {
            return err_json(StatusCode::FORBIDDEN, "Invalid token or IP address");
        };
        if file.token != q.token || file.done {
            return err_json(StatusCode::FORBIDDEN, "Invalid token or IP address");
        }
        file.attempts += 1;
        (
            file.file_name.clone(),
            file.size,
            file.sha256.clone(),
            file.mime.clone(),
            file.preview.clone(),
        )
    };
    {
        // 会话续命：一次上传（含重试）都算活动
        let mut slot = state.session.lock().await;
        if let Some(s) = slot.as_mut() {
            s.last_active = tokio::time::Instant::now();
        }
    }

    *state.rx_name.lock().await = file_name.clone();
    state.rx_total.store(size, Ordering::Relaxed);
    state.rx_bytes.store(0, Ordering::Relaxed);

    let final_path = state.resolve_path(&file_name).await;
    let part_path = {
        let mut p = final_path.clone().into_os_string();
        p.push(".part");
        std::path::PathBuf::from(p)
    };
    std::fs::create_dir_all(state.download_dir.read().await.as_os_str()).ok();
    let Ok(mut out) = tokio::fs::File::create(&part_path).await else {
        return err_json(StatusCode::INTERNAL_SERVER_ERROR, "无法创建临时文件");
    };

    let mut stream = req.into_body().into_data_stream();
    let mut hasher = Sha256::new();
    let mut got: u64 = 0;
    let mut io_err: Option<String> = None;
    while let Some(chunk) = stream.next().await {
        match chunk {
            Ok(bytes) => {
                hasher.update(&bytes);
                state.rx_bytes.fetch_add(bytes.len() as u64, Ordering::Relaxed);
                got += bytes.len() as u64;
                if out.write_all(&bytes).await.is_err() {
                    io_err = Some("写盘失败".into());
                    break;
                }
            }
            Err(e) => {
                io_err = Some(format!("接收中断: {e}"));
                break;
            }
        }
    }
    let _ = out.flush().await;
    let _ = out.sync_all().await;

    let sha_got = hex::encode(hasher.finalize());
    let mut bad = io_err.is_some() || got != size;
    if !bad {
        if let Some(expect) = &sha_expect {
            if !expect.eq_ignore_ascii_case(&sha_got) {
                bad = true;
            }
        }
    }

    if bad {
        let _ = tokio::fs::remove_file(&part_path).await;
        // 校验失败且未超次：保持会话可重试（发送方用同一 token 重传）
        let mut slot = state.session.lock().await;
        if let Some(session) = slot.as_mut() {
            if let Some(f) = session.files.get_mut(&q.file_id) {
                if f.attempts >= 3 {
                    f.done = true;
                }
            }
        }
        state
            .log_event(format!("接收 {} 失败（{}/{} 字节{}）", file_name, got, size,
                if sha_expect.is_some() && got == size { "，SHA-256 不一致" } else { "" }))
            .await;
        return err_json(StatusCode::UNPROCESSABLE_ENTITY, "Checksum mismatch");
    }

    // 文字消息识别：官方 LocalSend 的「发送文本」= text/plain 小文件，命名 <uuid>.txt，
    // 并把正文嵌进 FileDto.preview（消息与文件混发时 preview 缺席，以 uuid.txt 命名兜底）。
    // 普通文件名的 .md/.txt/.csv… 一律是真文件，照常落盘为文件卡片，
    // 不能把文档正文当聊天内容渲染。
    const TEXT_MSG_MAX: u64 = 64 * 1024;
    let stem = std::path::Path::new(&file_name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    let uuid_named = stem.len() == 36 && uuid::Uuid::parse_str(stem).is_ok();
    let is_message = mime.eq_ignore_ascii_case("text/plain")
        && got > 0
        && got <= TEXT_MSG_MAX
        && (!preview.is_empty() || uuid_named);
    let as_text: Option<String> = if is_message {
        tokio::fs::read(&part_path)
            .await
            .ok()
            .and_then(|b| {
                let s = String::from_utf8_lossy(&b).trim().to_string();
                (!s.is_empty()).then_some(s)
            })
    } else {
        None
    };

    let file_name_final = if as_text.is_some() {
        let _ = tokio::fs::remove_file(&part_path).await;
        String::new()
    } else {
        if tokio::fs::rename(&part_path, &final_path).await.is_err() {
            let _ = tokio::fs::remove_file(&part_path).await;
            return err_json(StatusCode::INTERNAL_SERVER_ERROR, "落盘失败");
        }
        final_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| file_name.clone())
    };
    // 会话流归因：优先 prepare 时记下的发送方指纹（同机测试/NAT 下 IP 对不上），
    // 缺失再按来源 IP 反查设备表，仍查不到用 IP 字符串占位（气泡不丢）
    let (sender_alias, fp_hint) = {
        let slot = state.session.lock().await;
        let s = slot.as_ref();
        (
            s.map(|s| s.sender_alias.clone()).unwrap_or_default(),
            s.map(|s| s.sender_fp.clone()).unwrap_or_default(),
        )
    };
    let sender_fp = if !fp_hint.is_empty() {
        fp_hint
    } else {
        state
            .devices
            .lock()
            .await
            .values()
            .find(|d| d.ip == peer.ip())
            .map(|d| d.fingerprint.clone())
            .unwrap_or_else(|| peer.ip().to_string())
    };
    if let Some(content) = as_text {
        state
            .push_chat(crate::state::ChatMsg {
                out: false,
                peer: sender_fp,
                peer_alias: sender_alias,
                kind: "text".into(),
                text: content,
                at: now_ms(),
                ..Default::default()
            })
            .await;
        state.log_event("收到一段文字消息".to_string()).await;
    } else {
        state
            .received
            .lock()
            .await
            .push(ReceivedFile {
                name: file_name_final.clone(),
                size: got,
                at: now_ms(),
                file: file_name_final.clone(),
            });
        state
            .push_chat(crate::state::ChatMsg {
                out: false,
                peer: sender_fp,
                peer_alias: sender_alias,
                kind: "file".into(),
                name: file_name_final.clone(),
                size: got,
                at: now_ms(),
                file: file_name_final.clone(),
                ..Default::default()
            })
            .await;
        state
            .log_event(format!("已接收 {}（{} 字节，来自本会话）", file_name_final, got))
            .await;
    }

    // 标记完成；全部完成则会话结束
    let all_done = {
        let mut slot = state.session.lock().await;
        let done = slot.as_mut().map(|s| {
            if let Some(f) = s.files.get_mut(&q.file_id) {
                f.done = true;
            }
            s.files.values().all(|f| f.done)
        });
        if done == Some(true) {
            let sender = slot.as_ref().map(|s| s.sender_alias.clone()).unwrap_or_default();
            drop(slot);
            state.log_event(format!("「{}」的传输会话完成", sender)).await;
            *state.session.lock().await = None;
            true
        } else {
            false
        }
    };
    let _ = all_done;

    StatusCode::OK.into_response()
}

#[derive(Deserialize)]
struct CancelQuery {
    #[serde(rename = "sessionId")]
    _session_id: Option<String>,
}

async fn cancel(State(state): State<Shared>, Query(_q): Query<CancelQuery>) -> Response {
    let had = state.session.lock().await.take().is_some();
    if had {
        state.log_event("对方取消了传输会话").await;
    }
    StatusCode::OK.into_response()
}

// ═══════════════ 面板 API ═══════════════

async fn dashboard() -> Html<&'static str> {
    Html(crate::ui::DASHBOARD)
}

/// 在线判定：多播周期 120s，300s 未见视为离线（列表仍保留，只是置灰）
const ONLINE_MS: u64 = 300_000;

async fn ui_state(State(state): State<Shared>) -> Response {
    let id = &state.identity;
    let devices: Vec<serde_json::Value> = {
        let devices = state.devices.lock().await;
        let now = now_ms();
        let online = |d: &Device| now.saturating_sub(d.last_seen) < ONLINE_MS;
        let mut list: Vec<&Device> = devices.values().collect();
        // 在线组（别名升序）在前，离线组（最近可见在前）在后
        list.sort_by(|a, b| match (online(a), online(b)) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            (true, true) => a.alias.cmp(&b.alias),
            (false, false) => b.last_seen.cmp(&a.last_seen),
        });
        list.iter()
            .map(|d| {
                json!({
                    "fingerprint": d.fingerprint,
                    "alias": d.alias,
                    "ip": d.ip.to_string(),
                    "addr": format!("{}:{}", d.ip, d.port),
                    "https": d.https,
                    "deviceType": d.device_type.clone().unwrap_or_else(|| "?".into()),
                    "version": d.version,
                    "lastSeenMs": now.saturating_sub(d.last_seen),
                    "online": online(d),
                })
            })
            .collect()
    };
    let received: Vec<serde_json::Value> = {
        let rec = state.received.lock().await;
        rec.iter()
            .rev()
            .take(60)
            .map(|r| json!({ "name": r.name, "size": r.size, "at": r.at, "file": r.file }))
            .collect()
    };
    let events: Vec<serde_json::Value> = {
        let ev = state.events.lock().await;
        ev.iter()
            .rev()
            .take(40)
            .map(|(t, s)| json!({ "t": t, "text": s }))
            .collect()
    };
    let session: serde_json::Value = {
        let slot = state.session.lock().await;
        match slot.as_ref() {
            Some(s) => json!({
                "active": true,
                "sender": s.sender_alias,
                "senderIp": s.sender_ip.to_string(),
                "current": {
                    "name": *state.rx_name.lock().await,
                    "got": state.rx_bytes.load(Ordering::Relaxed),
                    "total": state.rx_total.load(Ordering::Relaxed),
                },
                "files": s.files.values().map(|f| json!({
                    "name": f.file_name, "size": f.size, "done": f.done,
                })).collect::<Vec<_>>(),
            }),
            None => json!({"active": false}),
        }
    };
    let sending = state.sending.lock().await.clone();
    let chat: Vec<serde_json::Value> = state
        .db
        .recent_msgs(200)
        .iter()
        .map(|m| {
            json!({
                "id": m.id, "out": m.out, "peer": m.peer, "alias": m.peer_alias,
                "kind": m.kind, "text": m.text, "name": m.name, "size": m.size,
                "at": m.at, "file": m.file, "status": m.status, "srcPath": m.src_path,
            })
        })
        .collect();

    Json(json!({
        "me": {
            "alias": id.alias,
            "fingerprint": id.fingerprint,
            "port": id.port,
            "version": crate::config::PROTOCOL_VERSION,
            "appVersion": env!("CARGO_PKG_VERSION"),
            "dir": state.download_dir.read().await.display().to_string(),
        },
        "devices": devices,
        "received": received,
        "events": events,
        "session": session,
        "sending": sending,
        "chat": chat,
    }))
    .into_response()
}

#[derive(Deserialize)]
struct UiSendQuery {
    target: String,
    name: Option<String>,
    size: Option<u64>,
    mime: Option<String>,
}

/// 面板中转发送：浏览器把文件流式 POST 给本端，本端边收边转给目标设备（不落盘）
async fn ui_send(
    State(state): State<Shared>,
    Query(q): Query<UiSendQuery>,
    req: Request,
) -> Response {
    let Some(target) = state.devices.lock().await.get(&q.target).cloned() else {
        return err_json(StatusCode::NOT_FOUND, "目标设备不存在（可能已下线）");
    };
    let client = http_client();

    let name = q.name.unwrap_or_else(|| "file.bin".to_string());
    let size = q.size.unwrap_or(0);
    let mime = q.mime.unwrap_or_else(|| "application/octet-stream".to_string());

    // 先落一条 sending 消息（会话流立即出现 ⏳ 气泡；失败/成功只改状态，气泡不消失）。
    // 流式中转拿不到源路径 → src_path 空，失败后无重试按钮（Step 5 说明文案）
    let msg_id = state
        .push_chat(crate::state::ChatMsg {
            out: true,
            peer: target.fingerprint.clone(),
            peer_alias: target.alias.clone(),
            kind: "file".into(),
            name: name.clone(),
            size,
            at: now_ms(),
            status: "sending".into(),
            ..Default::default()
        })
        .await;

    // 1) prepare-upload（10s 总时限：对端不在线时 ⏳ 约 5~10s 变 ⚠，不再挂几十秒）
    let prepare = json!({
        "info": state.identity.register_json(),
        "files": { "f0": { "id": "f0", "fileName": name, "size": size, "fileType": mime } },
    });
    let resp = match client
        .post(format!("{}/api/localsend/v2/prepare-upload", target.base_url()))
        .timeout(std::time::Duration::from_secs(10))
        .json(&prepare)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            state.set_msg_status(msg_id, "fail").await;
            state.log_event(format!("发送 {name} 失败：联系「{}」失败: {e}", target.alias)).await;
            return err_json(StatusCode::BAD_GATEWAY, &format!("联系「{}」失败: {e}", target.alias));
        }
    };
    if !resp.status().is_success() {
        state.set_msg_status(msg_id, "fail").await;
        return err_json(StatusCode::BAD_GATEWAY, &format!("prepare-upload 返回 {}", resp.status()));
    }
    let parsed: serde_json::Value = match resp.json().await {
        Ok(v) => v,
        Err(_) => {
            state.set_msg_status(msg_id, "fail").await;
            return err_json(StatusCode::BAD_GATEWAY, "prepare-upload 响应异常");
        }
    };
    let session_id = parsed.get("sessionId").and_then(|v| v.as_str()).unwrap_or("");
    let token = parsed
        .pointer("/files/f0")
        .and_then(|v| v.as_str())
        .or_else(|| parsed.pointer("/files/f0/token").and_then(|v| v.as_str()))
        .unwrap_or("");
    if session_id.is_empty() || token.is_empty() {
        state.set_msg_status(msg_id, "fail").await;
        return err_json(StatusCode::BAD_GATEWAY, "对方未接受文件");
    }

    // 2) 边收边转
    let upload_url = format!(
        "{}/api/localsend/v2/upload?sessionId={session_id}&fileId=f0&token={token}",
        target.base_url()
    );
    {
        let mut p = state.sending.lock().await;
        *p = crate::state::SendProgress {
            active: true,
            target_alias: target.alias.clone(),
            file_name: name.clone(),
            sent: 0,
            total: size,
            started_at: now_ms(),
        };
    }
    let base_bytes = state.relay_bytes.load(Ordering::Relaxed);
    let tick_state = state.clone();
    let ticker = tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            let mut p = tick_state.sending.lock().await;
            if !p.active {
                break;
            }
            p.sent = tick_state
                .relay_bytes
                .load(Ordering::Relaxed)
                .saturating_sub(base_bytes);
        }
    });

    // 图片顺带留一份在内存（单张有上限），送达后会话流里可直接回显缩略图
    const IMG_SPILL_MAX: u64 = 8 * 1024 * 1024;
    let keep_img = mime.starts_with("image/") && size > 0 && size <= IMG_SPILL_MAX;
    let spill = Arc::new(std::sync::Mutex::new(Vec::with_capacity(
        if keep_img { size as usize } else { 0 },
    )));

    let counter_state = state.clone();
    let spill_buf = spill.clone();
    let stream = req
        .into_body()
        .into_data_stream()
        .map(move |chunk| {
            if let Ok(ref b) = chunk {
                counter_state
                    .relay_bytes
                    .fetch_add(b.len() as u64, Ordering::Relaxed);
                if keep_img {
                    if let Ok(mut v) = spill_buf.lock() {
                        v.extend_from_slice(b);
                    }
                }
            }
            chunk
        });
    let result = client
        .post(&upload_url)
        .header("Content-Type", &mime)
        .body(reqwest::Body::wrap_stream(stream))
        .send()
        .await;
    ticker.abort();
    *state.sending.lock().await = Default::default();

    match result {
        Ok(r) if r.status().is_success() => {
            state.set_msg_status(msg_id, "ok").await;
            state.log_event(format!("已送达「{}」：{}", target.alias, name)).await;
            if keep_img {
                // 先取走缓冲再 await：MutexGuard 非 Send，不能跨 await 持有
                let taken = spill
                    .lock()
                    .ok()
                    .filter(|v| !v.is_empty())
                    .map(|mut v| std::mem::take(&mut *v));
                if let Some(bytes) = taken {
                    state.cache_image(msg_id, mime.clone(), bytes).await;
                }
            }
            Json(json!({"ok": true})).into_response()
        }
        Ok(r) => {
            state.set_msg_status(msg_id, "fail").await;
            let msg = format!("对方返回 {}", r.status());
            state.log_event(format!("发送 {} 失败：{msg}", name)).await;
            err_json(StatusCode::BAD_GATEWAY, &msg)
        }
        Err(e) => {
            state.set_msg_status(msg_id, "fail").await;
            state.log_event(format!("发送 {} 失败: {e}", name)).await;
            err_json(StatusCode::BAD_GATEWAY, &format!("上传失败: {e}"))
        }
    }
}

#[derive(Deserialize)]
struct UiSendText {
    target: String,
    text: String,
}

async fn ui_send_text(State(state): State<Shared>, axum::Json(body): axum::Json<UiSendText>) -> Response {
    let Some(target) = state.devices.lock().await.get(&body.target).cloned() else {
        return err_json(StatusCode::NOT_FOUND, "目标设备不存在");
    };
    // 先落 sending 消息（⏳ 立即可见），失败置 ⚠（会话流留痕，无需回填输入框）
    let msg_id = state
        .push_chat(crate::state::ChatMsg {
            out: true,
            peer: target.fingerprint.clone(),
            peer_alias: target.alias.clone(),
            kind: "text".into(),
            text: body.text.clone(),
            at: now_ms(),
            status: "sending".into(),
            ..Default::default()
        })
        .await;
    let item = crate::client::item_from_text(body.text.clone());
    match crate::client::send(&state, &http_client(), &target, vec![item]).await {
        Ok(()) => {
            state.set_msg_status(msg_id, "ok").await;
            Json(json!({"ok": true})).into_response()
        }
        Err(e) => {
            state.set_msg_status(msg_id, "fail").await;
            err_json(StatusCode::BAD_GATEWAY, &format!("{e:#}"))
        }
    }
}

#[derive(Deserialize)]
struct UiAdd {
    ip: String,
    #[serde(default = "default_port")]
    port: u16,
}

/// 手动添加设备：GET 对方 /info 拿指纹与别名
async fn ui_add(State(state): State<Shared>, axum::Json(body): axum::Json<UiAdd>) -> Response {
    let url = format!("https://{}:{}/api/localsend/v2/info", body.ip, body.port);
    match http_client().get(&url).timeout(std::time::Duration::from_secs(5)).send().await {
        Ok(r) if r.status().is_success() => match r.json::<serde_json::Value>().await {
            Ok(v) => {
                let fingerprint = v
                    .get("fingerprint")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
                if fingerprint.is_empty() {
                    return err_json(StatusCode::BAD_GATEWAY, "对方未返回指纹");
                }
                let device = Device {
                    fingerprint: fingerprint.clone(),
                    alias: v.get("alias").and_then(|x| x.as_str()).unwrap_or("未知").to_string(),
                    ip: body.ip.parse().unwrap_or(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)),
                    port: body.port,
                    https: true,
                    device_model: v.get("deviceModel").and_then(|x| x.as_str()).map(str::to_string),
                    device_type: v.get("deviceType").and_then(|x| x.as_str()).map(str::to_string),
                    version: v.get("version").and_then(|x| x.as_str()).unwrap_or("?").to_string(),
                    download: v.get("download").and_then(|x| x.as_bool()).unwrap_or(false),
                    last_seen: now_ms(),
                };
                let alias = device.alias.clone();
                state.upsert_device(device).await;
                state.log_event(format!("手动添加设备「{alias}」")).await;
                Json(json!({"ok": true, "alias": alias})).into_response()
            }
            Err(_) => err_json(StatusCode::BAD_GATEWAY, "对方 info 响应异常"),
        },
        Ok(r) => err_json(StatusCode::BAD_GATEWAY, &format!("对方返回 {}", r.status())),
        Err(e) => err_json(StatusCode::BAD_GATEWAY, &format!("连不上 {url}: {e}")),
    }
}

#[derive(Deserialize)]
struct UiRemoveDevice {
    fingerprint: String,
}

/// 从列表移除设备（内存 + DB；消息历史保留，对方再上线自动回来）
async fn ui_remove_device(State(state): State<Shared>, axum::Json(body): axum::Json<UiRemoveDevice>) -> Response {
    if state.devices.lock().await.get(&body.fingerprint).is_none() {
        return err_json(StatusCode::NOT_FOUND, "设备不存在");
    }
    state.remove_device(&body.fingerprint).await;
    Json(json!({"ok": true})).into_response()
}

#[derive(Deserialize)]
struct UiPick {
    kind: String, // "file" | "folder"
}

/// 弹原生文件/文件夹选择框。GUI（Inline）在工作线程直接调 rfd；
/// CLI（Bridge）把任务转给主线程。返回 {paths:[…]}，用户取消为 {paths:null}。
/// 环境不支持（panic 被 catch）返回 501，前端回退浏览器 <input>。
async fn ui_pick(State(state): State<Shared>, axum::Json(body): axum::Json<UiPick>) -> Response {
    let folder = match body.kind.as_str() {
        "file" => false,
        "folder" => true,
        _ => return err_json(StatusCode::BAD_REQUEST, "kind 须为 file 或 folder"),
    };
    let picker = state.picker.lock().await;
    match &*picker {
        crate::state::Picker::Bridge(tx) => {
            let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
            if tx.send(crate::state::PickJob { folder, reply: reply_tx }).is_err() {
                return err_json(StatusCode::SERVICE_UNAVAILABLE, "主线程选择器通道已关闭");
            }
            match reply_rx.await {
                Ok(paths) => Json(json!({ "paths": paths })).into_response(),
                Err(_) => err_json(StatusCode::INTERNAL_SERVER_ERROR, "选择器无响应"),
            }
        }
        crate::state::Picker::Inline => {
            // macOS 无 NSApp 时 rfd 会 panic → catch_unwind 后告知前端走兜底
            let res = tokio::task::spawn_blocking(move || {
                std::panic::catch_unwind(move || {
                    if folder {
                        rfd::FileDialog::new().pick_folder().map(|p| vec![p])
                    } else {
                        rfd::FileDialog::new().pick_files()
                    }
                })
            })
            .await;
            match res {
                Ok(Ok(paths)) => Json(json!({ "paths": paths })).into_response(),
                Ok(Err(_)) => err_json(StatusCode::NOT_IMPLEMENTED, "此环境不支持原生选择器"),
                Err(e) => err_json(StatusCode::INTERNAL_SERVER_ERROR, &format!("选择器任务失败: {e}")),
            }
        }
    }
}

/// 路径展开：单文件 → [该文件]；目录 → 递归收集常规文件（跳过 . 开头段，含 .DS_Store/.git）
fn collect_files(path: &std::path::Path) -> anyhow::Result<Vec<std::path::PathBuf>> {
    let meta = std::fs::metadata(path).map_err(|e| anyhow::anyhow!("读取 {}: {e}", path.display()))?;
    if meta.is_file() {
        return Ok(vec![path.to_path_buf()]);
    }
    if !meta.is_dir() {
        anyhow::bail!("{} 不是文件或目录", path.display());
    }
    fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) -> anyhow::Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            if entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            let p = entry.path();
            if p.is_dir() {
                walk(&p, out)?;
            } else if p.is_file() {
                out.push(p);
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(path, &mut out)?;
    out.sort();
    Ok(out)
}

#[derive(Deserialize)]
struct UiSendPath {
    target: String,
    path: String,
}

/// 按本机路径发送（原生选择器返回的路径，或重试）：目录递归展开为多个独立气泡，
/// 每条消息记录 src_path（重试从原路径重读重发，不复制缓存副本）
async fn ui_send_path(State(state): State<Shared>, axum::Json(body): axum::Json<UiSendPath>) -> Response {
    let Some(target) = state.devices.lock().await.get(&body.target).cloned() else {
        return err_json(StatusCode::NOT_FOUND, "目标设备不存在（可能已被移除）");
    };
    let root = std::path::PathBuf::from(&body.path);
    let files = match collect_files(&root) {
        Ok(f) if f.is_empty() => return err_json(StatusCode::BAD_REQUEST, "没有可发送的文件"),
        Ok(f) if f.len() > 1000 => {
            return err_json(StatusCode::BAD_REQUEST, &format!("{} 个文件太多了（上限 1000）", f.len()))
        }
        Ok(f) => f,
        Err(e) => return err_json(StatusCode::BAD_REQUEST, &format!("{e:#}")),
    };

    let client = http_client();
    let mut ids = Vec::new();
    let mut first_err: Option<String> = None;
    for f in files {
        // 先建 item（含 SHA-256 与真实 size）再落气泡
        let item = match crate::client::item_from_path(f.clone()).await {
            Ok(i) => i,
            Err(e) => {
                if first_err.is_none() {
                    first_err = Some(format!("{e:#}"));
                }
                continue;
            }
        };
        let msg_id = state
            .push_chat(crate::state::ChatMsg {
                out: true,
                peer: target.fingerprint.clone(),
                peer_alias: target.alias.clone(),
                kind: "file".into(),
                name: item.file_name.clone(),
                size: item.size,
                at: now_ms(),
                status: "sending".into(),
                src_path: f.to_string_lossy().to_string(),
                ..Default::default()
            })
            .await;
        ids.push(msg_id);
        match send_item(&state, &client, &target, item, Some(&f), msg_id).await {
            Ok(()) => state.set_msg_status(msg_id, "ok").await,
            Err(e) => {
                state.set_msg_status(msg_id, "fail").await;
                if first_err.is_none() {
                    first_err = Some(e);
                }
            }
        }
    }
    match first_err {
        None => Json(json!({ "ok": true, "ids": ids })).into_response(),
        Some(e) => {
            state.log_event(format!("路径发送部分失败：{e}")).await;
            Json(json!({ "ok": false, "ids": ids, "error": e })).into_response()
        }
    }
}

#[derive(Deserialize)]
struct UiRetry {
    id: u64,
}

/// 重试未送达的出站消息：文本重发正文；文件从 src_path 原路径重读（源没了 → 保持 ⚠）。
/// 发送时间 at 不动（气泡位置不变），仅状态流转
async fn ui_retry(State(state): State<Shared>, axum::Json(body): axum::Json<UiRetry>) -> Response {
    let Some(m) = state.db.get_msg(body.id) else {
        return err_json(StatusCode::NOT_FOUND, "消息不存在");
    };
    if !m.out {
        return err_json(StatusCode::BAD_REQUEST, "收到的消息无需重试");
    }
    if m.status == "sending" {
        return err_json(StatusCode::CONFLICT, "该消息正在发送中");
    }
    if m.status != "fail" {
        return err_json(StatusCode::BAD_REQUEST, "该消息已送达");
    }
    let Some(target) = state.devices.lock().await.get(&m.peer).cloned() else {
        return err_json(StatusCode::BAD_REQUEST, "设备已从列表移除，无法重试");
    };
    state.set_msg_status(m.id, "sending").await;
    let client = http_client();
    let res = if m.kind == "text" {
        let item = crate::client::item_from_text(m.text.clone());
        crate::client::send(&state, &client, &target, vec![item])
            .await
            .map_err(|e| format!("{e:#}"))
    } else if !m.src_path.is_empty() {
        let p = std::path::PathBuf::from(&m.src_path);
        match crate::client::item_from_path(p.clone()).await {
            Ok(item) => send_item(&state, &client, &target, item, Some(&p), m.id).await,
            Err(e) => Err(format!("{e:#}")), // 源文件已删/不可读：状态保持 fail
        }
    } else {
        // 流式中转无源路径——正常 UI 不给按钮，防御分支
        state.set_msg_status(m.id, "fail").await;
        return err_json(StatusCode::BAD_REQUEST, "该消息没有源文件可重试（请重新拖入）");
    };
    match res {
        Ok(()) => {
            state.set_msg_status(m.id, "ok").await;
            state.log_event(format!("重试成功：{}", if m.kind == "text" { "文字消息" } else { &m.name })).await;
            Json(json!({ "ok": true })).into_response()
        }
        Err(e) => {
            state.set_msg_status(m.id, "fail").await;
            err_json(StatusCode::BAD_GATEWAY, &e)
        }
    }
}

/// 发一个已构造好的 item（send-path / retry 共用）→ client::send。
/// 成功且 cache_src 指向 ≤8MB 图片时读盘回显进内存缓存
async fn send_item(
    state: &Shared,
    client: &reqwest::Client,
    target: &crate::state::Device,
    item: crate::client::SendItem,
    cache_src: Option<&std::path::Path>,
    msg_id: u64,
) -> Result<(), String> {
    const IMG_SPILL_MAX: u64 = 8 * 1024 * 1024;
    let is_img = cache_src.is_some()
        && item.mime.starts_with("image/")
        && item.size > 0
        && item.size <= IMG_SPILL_MAX;
    let mime = item.mime.clone();
    match crate::client::send(state, client, target, vec![item]).await {
        Ok(()) => {
            if is_img {
                if let Ok(bytes) = tokio::fs::read(cache_src.unwrap()).await {
                    state.cache_image(msg_id, mime, bytes).await;
                }
            }
            Ok(())
        }
        Err(e) => Err(format!("{e:#}")),
    }
}

#[derive(Deserialize)]
struct UiReveal {
    file: String,
}

async fn ui_reveal(State(state): State<Shared>, axum::Json(body): axum::Json<UiReveal>) -> Response {
    let dl = state.download_dir.read().await.clone();
    let dir = dl.canonicalize().unwrap_or(dl);
    let path = dir.join(&body.file);
    // 必须真实存在才能 canonicalize 成功；解析失败（含 .. 穿越、不存在）一律拒绝，
    // 之后 starts_with 在两条已解析路径上比较，不可被词法绕过
    let Ok(canon) = path.canonicalize() else {
        return err_json(StatusCode::FORBIDDEN, "路径越界");
    };
    if !canon.starts_with(&dir) {
        return err_json(StatusCode::FORBIDDEN, "路径越界");
    }
    #[cfg(target_os = "macos")]
    let ok = std::process::Command::new("open").arg("-R").arg(&canon).spawn().is_ok();
    #[cfg(target_os = "windows")]
    let ok = std::process::Command::new("explorer")
        .arg(format!("/select,{}", canon.display()))
        .spawn()
        .is_ok();
    #[cfg(all(unix, not(target_os = "macos")))]
    let ok = std::process::Command::new("xdg-open")
        .arg(canon.parent().unwrap_or(&canon))
        .spawn()
        .is_ok();
    #[cfg(not(any(unix, target_os = "windows")))]
    let ok = false;
    if ok {
        Json(json!({"ok": true})).into_response()
    } else {
        err_json(StatusCode::INTERNAL_SERVER_ERROR, "无法打开文件位置")
    }
}

#[derive(Deserialize)]
struct UiAssetQuery {
    file: Option<String>,
    id: Option<u64>,
}

/// 图片扩展名 → Content-Type（白名单外的扩展名一律不伺服；
/// svg 可携带脚本，不列入）
fn img_content_type(name: &str) -> Option<&'static str> {
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    Some(match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" | "jfif" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "heic" | "heif" => "image/heic",
        "avif" => "image/avif",
        _ => return None,
    })
}

/// 会话流图片源：?file= 收到的图（限下载目录内、限图片扩展名）；
/// ?id= 出站图（内存缓存，淘汰后 404，前端退回文件卡片）。
/// 同一 URL 内容不变，允许 WebView 缓存，避免气泡重绘反复拉取。
async fn ui_asset(State(state): State<Shared>, Query(q): Query<UiAssetQuery>) -> Response {
    if let Some(id) = q.id {
        let hit = state
            .img_cache
            .lock()
            .await
            .get(&id)
            .map(|i| (i.mime.clone(), i.bytes.clone()));
        if let Some((mime, bytes)) = hit {
            return (
                [
                    (header::CONTENT_TYPE, mime),
                    (header::CACHE_CONTROL, "private, max-age=86400".to_string()),
                ],
                bytes,
            )
                .into_response();
        }
        return err_json(StatusCode::NOT_FOUND, "预览已过期");
    }
    let Some(name) = q.file else {
        return err_json(StatusCode::BAD_REQUEST, "缺少 file");
    };
    let Some(ct) = img_content_type(&name) else {
        return err_json(StatusCode::FORBIDDEN, "仅支持图片文件");
    };
    let dl = state.download_dir.read().await.clone();
    let dir = dl.canonicalize().unwrap_or(dl);
    // 与 ui_reveal 同款防线：先解析再前缀比较，挡住 .. 穿越与不存在路径
    let Ok(canon) = dir.join(&name).canonicalize() else {
        return err_json(StatusCode::FORBIDDEN, "路径越界");
    };
    if !canon.starts_with(&dir) {
        return err_json(StatusCode::FORBIDDEN, "路径越界");
    }
    match tokio::fs::read(&canon).await {
        Ok(bytes) => (
            [
                (header::CONTENT_TYPE, ct.to_string()),
                (header::CACHE_CONTROL, "private, max-age=86400".to_string()),
            ],
            bytes,
        )
            .into_response(),
        Err(_) => err_json(StatusCode::NOT_FOUND, "文件不存在"),
    }
}

/// 面板/客户端共用的 reqwest：忽略自签证书、不走代理（纯内网）。
/// 只设连接超时（离线对端 5s 内失败）；总超时会掐断大文件上传，需总时限的
/// 单次请求在调用处用 RequestBuilder::timeout 另加
pub fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(5))
        .danger_accept_invalid_certs(true)
        .use_rustls_tls()
        .no_proxy()
        .build()
        .expect("reqwest client")
}

/// 供 main 使用的共享构造（打开 SQLite，失败即上层报错退出）
pub fn new_state(identity: crate::config::Identity) -> anyhow::Result<Shared> {
    let db = crate::db::Db::open(&identity.cfg_dir.join("chat.db"))?;
    let dl = identity.download_dir.clone();
    Ok(Arc::new(crate::state::AppState {
        identity,
        download_dir: tokio::sync::RwLock::new(dl),
        db,
        picker: tokio::sync::Mutex::new(crate::state::Picker::Inline),
        devices: Default::default(),
        session: Default::default(),
        received: Default::default(),
        events: Default::default(),
        sending: Default::default(),
        img_cache: Default::default(),
        relay_bytes: Default::default(),
        rx_bytes: Default::default(),
        rx_total: Default::default(),
        rx_name: Default::default(),
    }))
}

// ═══════════════ 设置面板 API ═══════════════

#[derive(Deserialize)]
struct UiSetDir {
    dir: String,
}

/// 改保存目录：建目录（不存在则创建）→ 可写探测 → 写 config.json → 换内存值。
/// reveal/asset 的越界防线按解析后路径比较，这里先 canonicalize，展示与防线一致
async fn ui_set_dir(State(state): State<Shared>, axum::Json(body): axum::Json<UiSetDir>) -> Response {
    if body.dir.trim().is_empty() {
        return err_json(StatusCode::BAD_REQUEST, "目录不能为空");
    }
    if let Err(e) = std::fs::create_dir_all(&body.dir) {
        return err_json(StatusCode::BAD_REQUEST, &format!("目录不可用: {e}"));
    }
    let dir = match std::path::PathBuf::from(&body.dir).canonicalize() {
        Ok(d) => d,
        Err(e) => return err_json(StatusCode::BAD_REQUEST, &format!("目录不可用: {e}")),
    };
    // 可写探测：临时文件建了就删
    let probe = dir.join(format!(".antify-probe-{}", uuid::Uuid::new_v4()));
    if let Err(e) = std::fs::File::create(&probe).and_then(|_| std::fs::remove_file(&probe)) {
        return err_json(StatusCode::BAD_REQUEST, &format!("目录不可写: {e}"));
    }
    if let Err(e) = crate::config::save_download_dir(&dir) {
        return err_json(StatusCode::INTERNAL_SERVER_ERROR, &format!("保存失败: {e:#}"));
    }
    *state.download_dir.write().await = dir.clone();
    state.log_event(format!("保存目录已改为 {}", dir.display())).await;
    Json(json!({"ok": true, "dir": dir.display().to_string()})).into_response()
}

/// 版本比较：latest 每段数值大于 current 才算更新（v 前缀 / -pre 后缀容忍，段解析失败按 0）
fn version_newer(latest: &str, current: &str) -> bool {
    let nums = |s: &str| -> Vec<u64> {
        s.trim_start_matches('v')
            .split('-')
            .next()
            .unwrap_or("")
            .split('.')
            .map(|p| p.trim().parse().unwrap_or(0))
            .collect()
    };
    let (a, b) = (nums(latest), nums(current));
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0));
        if x != y {
            return x > y;
        }
    }
    false
}

/// 检查更新：GitHub Releases 最新 tag 与当前版本比较。
/// 与 http_client() 相反——这里必须走系统代理（国内直连不到 GitHub），
/// 且要带 User-Agent（GitHub API 强制要求）
async fn ui_check_update() -> Response {
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(8))
        .timeout(std::time::Duration::from_secs(15))
        .user_agent(format!("AntifyBot/{}", env!("CARGO_PKG_VERSION")))
        .use_rustls_tls()
        .build()
        .expect("reqwest client");
    let resp = match client
        .get("https://api.github.com/repos/robotbird/antifybot/releases/latest")
        .header(header::ACCEPT, "application/vnd.github+json")
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => return err_json(StatusCode::BAD_GATEWAY, &format!("检查更新失败: {e}")),
    };
    if !resp.status().is_success() {
        return err_json(StatusCode::BAD_GATEWAY, &format!("GitHub 返回 {}", resp.status()));
    }
    let Ok(v) = resp.json::<serde_json::Value>().await else {
        return err_json(StatusCode::BAD_GATEWAY, "GitHub 响应解析失败");
    };
    let tag = v.get("tag_name").and_then(|x| x.as_str()).unwrap_or("").to_string();
    if tag.is_empty() {
        return err_json(StatusCode::BAD_GATEWAY, "还没有已发布版本");
    }
    Json(json!({
        "current": env!("CARGO_PKG_VERSION"),
        "latest": tag,
        "newer": version_newer(&tag, env!("CARGO_PKG_VERSION")),
        "url": v.get("html_url").and_then(|x| x.as_str()).unwrap_or(""),
        "notes": v.get("body").and_then(|x| x.as_str()).unwrap_or(""),
    }))
    .into_response()
}

#[derive(Deserialize)]
struct UiOpenUrl {
    url: String,
}

/// 系统浏览器打开（WebView 里 window.open / target=_blank 不可靠）。
/// 仅放行本仓库 GitHub 页面——面板是唯一调用方，别让它变成任意打开器
async fn ui_open_url(axum::Json(body): axum::Json<UiOpenUrl>) -> Response {
    let allowed = [
        "https://github.com/robotbird/antifybot",
        "https://api.github.com/repos/robotbird/antifybot",
    ];
    if !allowed.iter().any(|a| body.url.starts_with(a)) {
        return err_json(StatusCode::FORBIDDEN, "仅允许打开 GitHub 仓库页面");
    }
    #[cfg(target_os = "macos")]
    let ok = std::process::Command::new("open").arg(&body.url).spawn().is_ok();
    #[cfg(target_os = "windows")]
    let ok = std::process::Command::new("cmd")
        .args(["/C", "start", "", &body.url])
        .spawn()
        .is_ok();
    #[cfg(all(unix, not(target_os = "macos")))]
    let ok = std::process::Command::new("xdg-open").arg(&body.url).spawn().is_ok();
    #[cfg(not(any(unix, target_os = "windows")))]
    let ok = false;
    if ok {
        Json(json!({"ok": true})).into_response()
    } else {
        err_json(StatusCode::INTERNAL_SERVER_ERROR, "无法打开浏览器")
    }
}
