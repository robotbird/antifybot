mod rtc;
pub(super) mod transfer;

use super::*;
use antify_protocol::{identity_proof, DirectEvent, DirectRequest, HTTPS_RELAY};
use base64::{engine::general_purpose::STANDARD, Engine};
use rtc::{Connection, Wire};
use tokio::{sync::mpsc, task::JoinSet};

fn pem_contents(pem: &str) -> String {
    pem.lines()
        .filter(|line| !line.starts_with("---"))
        .map(str::trim)
        .collect()
}

fn register(state: &Shared, device_id: &str, nonce: &str) -> Result<DirectRequest> {
    use ring::{
        rand::SystemRandom,
        signature::{EcdsaKeyPair, ECDSA_P256_SHA256_ASN1_SIGNING},
    };
    let certificate = pem_contents(&std::fs::read_to_string(&state.identity.cert_pem)?);
    let key = STANDARD.decode(pem_contents(&std::fs::read_to_string(
        &state.identity.key_pem,
    )?))?;
    let random = SystemRandom::new();
    let key = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &key, &random)
        .map_err(|_| anyhow::anyhow!("无法加载本机设备签名密钥"))?;
    let alias = state.identity.alias.clone();
    let proof = key
        .sign(
            &random,
            &identity_proof(device_id, nonce, &alias, &state.identity.fingerprint),
        )
        .map_err(|_| anyhow::anyhow!("无法签署设备身份"))?;
    Ok(DirectRequest::Register {
        alias,
        certificate,
        signature: STANDARD.encode(proof.as_ref()),
    })
}

type SignalWaiters = Mutex<HashMap<String, (String, tokio::sync::oneshot::Sender<Option<String>>)>>;

pub(super) async fn session(state: &Shared, cfg: &Settings) -> Result<()> {
    let url = format!("{}/v1/p2p", cfg.server_url.replacen("http", "ws", 1));
    let mut request = url.into_client_request()?;
    request
        .headers_mut()
        .insert("authorization", format!("Bearer {}", cfg.token).parse()?);
    let (mut socket, _) = tokio::time::timeout(Duration::from_secs(10), connect_async(request))
        .await?
        .context("无法连接点对点信令服务，请确认服务端已升级")?;
    let hello = tokio::time::timeout(Duration::from_secs(10), socket.next())
        .await?
        .context("信令服务未返回身份挑战")??;
    let DirectEvent::Challenge { device_id, nonce } = serde_json::from_str(hello.to_text()?)?
    else {
        bail!("信令握手异常");
    };
    socket
        .send(Frame::Text(
            serde_json::to_string(&register(state, &device_id, &nonce)?)?.into(),
        ))
        .await?;
    let (signals, mut outgoing) = mpsc::channel::<DirectRequest>(32);
    let waiters = std::sync::Arc::new(SignalWaiters::default());
    let mut incoming = JoinSet::<String>::new();
    let mut active_peers = HashMap::<String, usize>::new();
    let mut recent_sessions = HashMap::new();
    let stun =
        std::env::var("ANTIFY_P2P_STUN").unwrap_or_else(|_| "stun:stun.l.google.com:19302".into());
    ensure!(
        stun.is_empty() || stun.starts_with("stun:"),
        "仅支持 STUN 服务器"
    );
    let stun = (!stun.is_empty()).then_some(stun);
    let (mut writer, mut reader) = socket.split();
    let read = async {
        loop {
            tokio::select! {
                frame = tokio::time::timeout(Duration::from_secs(65), reader.next()) => {
                    let frame = frame?.context("信令连接已断开")??;
                    match frame {
                        Frame::Ping(data) => writer.send(Frame::Pong(data)).await?,
                        Frame::Close(_) => bail!("信令连接已关闭"),
                        Frame::Text(text) => match serde_json::from_str::<DirectEvent>(&text)? {
                            DirectEvent::Devices { devices } => {
                                let mut online = HashSet::new();
                                for device in devices.into_iter().filter(|p| p.id != device_id && p.fingerprint != state.identity.fingerprint) {
                                    match state.db.bind_direct_device(&cfg.scope(), &device) {
                                        Ok(()) => { online.insert(device.id); }
                                        Err(e) => eprintln!("P2P device binding rejected: {e}"),
                                    }
                                }
                                *state.relay.online.write().await = online;
                                let mut status = state.relay.status.write().await;
                                if !status.connected {
                                    *status = Status { connected:true, device_id:device_id.clone(), error:String::new() };
                                }
                            }
                            DirectEvent::Signal { from, session, offer: false, sdp } => {
                                let mut pending = waiters.lock().unwrap();
                                if pending.get(&session).is_some_and(|(id, _)| id == &from.id)
                                    && state.db.bind_direct_device(&cfg.scope(), &from).is_ok()
                                {
                                    let _ = pending.remove(&session).unwrap().1.send(Some(sdp));
                                }
                            }
                            DirectEvent::Rejected { session } => {
                                if let Some((_, tx)) = waiters.lock().unwrap().remove(&session) { let _ = tx.send(None); }
                            }
                            DirectEvent::Signal { from, session, offer: true, sdp } => {
                                recent_sessions.retain(|_, time: &mut std::time::Instant| time.elapsed() < Duration::from_secs(90));
                                if incoming.len() >= 4 || recent_sessions.len() >= 256 || recent_sessions.contains_key(&session)
                                    || active_peers.get(&from.fingerprint).copied().unwrap_or(0) >= 2
                                    || state.db.bind_direct_device(&cfg.scope(), &from).is_err() { continue; }
                                recent_sessions.insert(session.clone(), std::time::Instant::now());
                                *active_peers.entry(from.fingerprint.clone()).or_default() += 1;
                                let state = state.clone();
                                let cfg = cfg.clone();
                                let signals = signals.clone();
                                let stun = stun.clone();
                                incoming.spawn(async move {
                                    let start = std::time::Instant::now();
                                    if let Err(e) = accept(&state, &cfg, &from, session, sdp, stun.as_deref(), &signals).await {
                                        state.log_event(format!("点对点接收失败，耗时 {:.1} 秒：{e}", start.elapsed().as_secs_f64())).await;
                                    }
                                    from.fingerprint
                                });
                            }
                            DirectEvent::Challenge { .. } => bail!("重复身份挑战"),
                        },
                        _ => {}
                    }
                }
                Some(signal) = outgoing.recv() => {
                    tokio::time::timeout(Duration::from_secs(5), writer.send(Frame::Text(serde_json::to_string(&signal)?.into()))).await??;
                }
                Some(result) = incoming.join_next(), if !incoming.is_empty() => {
                    let peer = result?;
                    if let Some(count) = active_peers.get_mut(&peer) {
                        *count -= 1;
                        if *count == 0 { active_peers.remove(&peer); }
                    }
                }
            }
        }
        #[allow(unreachable_code)]
        Ok::<(), anyhow::Error>(())
    };
    tokio::try_join!(
        read,
        outbox(state, cfg, stun.as_deref(), &signals, &waiters, "text"),
        outbox(state, cfg, stun.as_deref(), &signals, &waiters, "file"),
    )?;
    Ok(())
}

async fn channel(pc: &Connection, opened: &mut mpsc::Receiver<Wire>) -> Result<Wire> {
    match tokio::time::timeout(Duration::from_secs(20), opened.recv()).await {
        Ok(Some(wire)) => Ok(wire),
        Ok(None) => bail!("点对点数据通道未建立"),
        Err(_) => bail!(
            "点对点连接超时（ICE: {}，连接: {}）",
            pc.0.ice_connection_state(),
            pc.0.connection_state()
        ),
    }
}

async fn relay_wire(cfg: &Settings, session: &str) -> Result<Wire> {
    let url = format!(
        "{}/v1/p2p?session={session}",
        cfg.server_url.replacen("http", "ws", 1)
    );
    let mut request = url.into_client_request()?;
    request
        .headers_mut()
        .insert("authorization", format!("Bearer {}", cfg.token).parse()?);
    let (socket, _) = tokio::time::timeout(Duration::from_secs(10), connect_async(request))
        .await?
        .context("无法连接 HTTPS 中转通道，请确认两端和服务器均已更新")?;
    Ok(Wire::relay(socket))
}

async fn accept(
    state: &Shared,
    cfg: &Settings,
    peer: &DirectDevice,
    session: String,
    sdp: String,
    stun: Option<&str>,
    signals: &mpsc::Sender<DirectRequest>,
) -> Result<()> {
    if sdp == HTTPS_RELAY {
        signals
            .send(DirectRequest::Signal {
                to: peer.id.clone(),
                session: session.clone(),
                offer: false,
                sdp,
            })
            .await?;
        let wire = relay_wire(cfg, &session).await?;
        return transfer::receive(state, cfg, peer, &wire).await;
    }
    let pc = Connection::new(stun).await?;
    let (tx, mut rx) = mpsc::channel(1);
    pc.0.on_data_channel(Box::new(move |dc| {
        if dc.label() == "antify-p2p-v1" {
            rtc::detach(dc, tx.clone());
        }
        Box::pin(async {})
    }));
    pc.remote(true, sdp).await?;
    let sdp = pc.description(false).await?;
    signals
        .send(DirectRequest::Signal {
            to: peer.id.clone(),
            session,
            offer: false,
            sdp,
        })
        .await?;
    let wire = channel(&pc, &mut rx).await?;
    if let Some(path) = pc.path().await {
        state.log_event(format!("点对点接收链路 {path}")).await;
    }
    transfer::receive(state, cfg, peer, &wire).await
}

async fn send_item(
    state: &Shared,
    cfg: &Settings,
    stun: Option<&str>,
    signals: &mpsc::Sender<DirectRequest>,
    waiters: &SignalWaiters,
    item: &RelayPending,
) -> Result<()> {
    let payload = serde_json::from_str::<Payload>(
        item.body
            .as_deref()
            .context("旧版文件任务，请重新选择文件")?,
    )
    .context("旧版文件任务，请重新选择文件")?;
    let source = transfer::open_source(item, &payload).await?;
    let mut direct = if cfg.prefer_p2p {
        let pc = Connection::new(stun).await?;
        let (tx, rx) = mpsc::channel(1);
        rtc::detach(pc.0.create_data_channel("antify-p2p-v1", None).await?, tx);
        Some((pc, rx))
    } else {
        None
    };
    let sdp = match &direct {
        Some((pc, _)) => pc.description(true).await?,
        None => HTTPS_RELAY.to_owned(),
    };
    let session = uuid::Uuid::new_v4().to_string();
    let (tx, answer) = tokio::sync::oneshot::channel();
    waiters
        .lock()
        .unwrap()
        .insert(session.clone(), (item.recipient.clone(), tx));
    let sent = signals
        .send(DirectRequest::Signal {
            to: item.recipient.clone(),
            session: session.clone(),
            offer: true,
            sdp,
        })
        .await;
    let reply = if sent.is_ok() {
        tokio::time::timeout(Duration::from_secs(25), answer)
            .await
            .ok()
    } else {
        None
    };
    waiters.lock().unwrap().remove(&session);
    let sdp = reply
        .and_then(Result::ok)
        .flatten()
        .context("无法建立点对点连接，对方未应答或已离线")?;
    let wire = if let Some((pc, rx)) = &mut direct {
        pc.remote(false, sdp).await?;
        let wire = channel(pc, rx).await?;
        if let Some(path) = pc.path().await {
            state.log_event(format!("点对点发送链路 {path}")).await;
        }
        wire
    } else {
        ensure!(sdp == HTTPS_RELAY, "对端不支持 HTTPS 中转");
        relay_wire(cfg, &session).await?
    };
    state
        .db
        .relay_status(&cfg.scope(), &item.remote_id, "sending")?;
    transfer::send(state, &wire, item, payload, source).await
}

struct ResetProgress(Shared);
impl Drop for ResetProgress {
    fn drop(&mut self) {
        *self.0.relay.progress.lock().unwrap() = Default::default();
    }
}

async fn outbox(
    state: &Shared,
    cfg: &Settings,
    stun: Option<&str>,
    signals: &mpsc::Sender<DirectRequest>,
    waiters: &SignalWaiters,
    kind: &str,
) -> Result<()> {
    let is_file = kind == "file";
    let mut tick = tokio::time::interval(Duration::from_secs(2));
    let mut resume_attempts = HashSet::new();
    loop {
        tick.tick().await;
        if !state.relay.status.read().await.connected {
            continue;
        }
        let online = state.relay.online.read().await.clone();
        for item in state.db.relay_pending(&cfg.scope(), &online, kind)? {
            if is_file {
                if state.sending.lock().await.active {
                    break;
                }
                state.send_cancel.store(false, Ordering::Relaxed);
            }
            state
                .db
                .relay_status(&cfg.scope(), &item.remote_id, "connecting")?;
            let _guard = is_file.then(|| ResetProgress(state.clone()));
            let start = std::time::Instant::now();
            match send_item(state, cfg, stun, signals, waiters, &item).await {
                Ok(()) => {
                    resume_attempts.remove(&item.remote_id);
                    if let Some((local, path)) =
                        state.db.relay_delivered(&cfg.scope(), &item.remote_id)?
                    {
                        state.set_msg_status(local, "ok").await;
                        if Path::new(&path).starts_with(state.identity.cfg_dir.join("relay-outbox"))
                        {
                            let _ = tokio::fs::remove_file(path).await;
                        }
                    }
                    state.relay.status.write().await.error.clear();
                    eprintln!("P2P sent after {:?}", start.elapsed());
                }
                Err(e) => {
                    if e.is::<rtc::Interrupted>() && resume_attempts.insert(item.remote_id.clone())
                    {
                        state
                            .db
                            .relay_status(&cfg.scope(), &item.remote_id, "resuming")?;
                        state.log_event("点对点连接中断，等待续传").await;
                        continue;
                    }
                    resume_attempts.remove(&item.remote_id);
                    state
                        .db
                        .direct_fail(&cfg.scope(), &item.remote_id, &e.to_string())?;
                    state.relay.status.write().await.error = format!("点对点发送失败，可重试：{e}");
                    state
                        .log_event(format!("点对点发送失败，可重试：{e}"))
                        .await;
                    eprintln!("P2P send failed after {:?}: {e}", start.elapsed());
                }
            }
        }
    }
}
