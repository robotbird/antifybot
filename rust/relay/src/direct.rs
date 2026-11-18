use super::*;
use antify_protocol::{
    direct_sdp, identity_proof, DirectDevice, DirectEvent, DirectRequest, HTTPS_RELAY,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use tokio::sync::mpsc;

#[derive(Default)]
pub(super) struct Registry(
    Mutex<HashMap<String, (DirectDevice, mpsc::Sender<DirectEvent>)>>,
    Mutex<HashMap<String, Pair>>,
);
struct Pair {
    from: String,
    to: String,
    expires: std::time::Instant,
    waiting: Option<(String, tokio::sync::oneshot::Sender<WebSocket>)>,
}
#[derive(Deserialize)]
pub(super) struct UpgradeQuery {
    session: Option<String>,
}

struct Registration(App, String);
impl Drop for Registration {
    fn drop(&mut self) {
        self.0 .0.direct.0.lock().unwrap().remove(&self.1);
    }
}

fn verify(
    id: String,
    nonce: &str,
    alias: String,
    certificate: &str,
    signature: &str,
) -> anyhow::Result<DirectDevice> {
    use anyhow::{anyhow, ensure};
    use x509_parser::prelude::*;
    ensure!(
        !alias.is_empty() && alias.len() <= 256 && !alias.chars().any(char::is_control),
        "invalid alias"
    );
    let der = STANDARD.decode(certificate)?;
    let (rest, cert) = X509Certificate::from_der(&der)?;
    ensure!(rest.is_empty(), "invalid certificate");
    let fingerprint = hex::encode_upper(Sha256::digest(&der));
    ring::signature::UnparsedPublicKey::new(
        &ring::signature::ECDSA_P256_SHA256_ASN1,
        cert.public_key().subject_public_key.data.as_ref(),
    )
    .verify(
        &identity_proof(&id, nonce, &alias, &fingerprint),
        &STANDARD.decode(signature)?,
    )
    .map_err(|_| anyhow!("invalid identity proof"))?;
    Ok(DirectDevice {
        id,
        alias,
        fingerprint,
    })
}

pub(super) async fn upgrade(
    State(app): State<App>,
    Auth(device): Auth,
    axum::extract::Query(query): axum::extract::Query<UpgradeQuery>,
    ws: WebSocketUpgrade,
) -> Response {
    if let Some(session) = query.session {
        let pairs = app.0.direct.1.lock().unwrap();
        if !pairs.get(&session).is_some_and(|p| {
            p.expires > std::time::Instant::now() && (device == p.from || device == p.to)
        }) {
            return StatusCode::NOT_FOUND.into_response();
        }
        drop(pairs);
        return ws
            .max_message_size(16 * 1024)
            .max_frame_size(16 * 1024)
            .on_upgrade(move |socket| relay(socket, app, session, device));
    }
    ws.max_message_size(64 * 1024)
        .max_frame_size(64 * 1024)
        .on_upgrade(move |socket| serve(socket, app, device))
}

async fn relay(socket: WebSocket, app: App, session: String, device: String) {
    use futures_util::{SinkExt, StreamExt};
    let waiting = {
        let mut pairs = app.0.direct.1.lock().unwrap();
        let Some(pair) = pairs.get_mut(&session) else {
            return;
        };
        if pair.expires <= std::time::Instant::now() || (device != pair.from && device != pair.to) {
            return;
        }
        if let Some((first, _)) = &pair.waiting {
            if first == &device {
                return;
            }
            let (_, tx) = pairs.remove(&session).unwrap().waiting.unwrap();
            let _ = tx.send(socket);
            return;
        }
        let (tx, rx) = tokio::sync::oneshot::channel();
        pair.waiting = Some((device, tx));
        rx
    };
    let Ok(Ok(peer)) = tokio::time::timeout(Duration::from_secs(15), waiting).await else {
        app.0.direct.1.lock().unwrap().remove(&session);
        return;
    };
    static ACTIVE_RELAYS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(16);
    let Ok(_permit) = ACTIVE_RELAYS.try_acquire() else {
        return;
    };
    let (mut first_tx, mut first_rx) = socket.split();
    let (mut peer_tx, mut peer_rx) = peer.split();
    macro_rules! forward {
        ($source:expr, $target:expr) => {
            async {
                while let Ok(Some(Ok(frame))) =
                    tokio::time::timeout(Duration::from_secs(120), $source.next()).await
                {
                    match frame {
                        WsMessage::Binary(data) => {
                            if !matches!(
                                tokio::time::timeout(
                                    Duration::from_secs(30),
                                    $target.send(WsMessage::Binary(data))
                                )
                                .await,
                                Ok(Ok(()))
                            ) {
                                break;
                            }
                        }
                        WsMessage::Close(_) | WsMessage::Text(_) => break,
                        _ => {}
                    }
                }
            }
        };
    }
    tokio::select! {
        _ = forward!(first_rx, peer_tx) => {},
        _ = forward!(peer_rx, first_tx) => {},
    }
}

async fn send(socket: &mut WebSocket, event: &DirectEvent) -> anyhow::Result<()> {
    let data = serde_json::to_string(event)?;
    tokio::time::timeout(
        Duration::from_secs(5),
        socket.send(WsMessage::Text(data.into())),
    )
    .await??;
    Ok(())
}

async fn serve(mut socket: WebSocket, app: App, id: String) {
    let _ = connection(&mut socket, app, id).await;
}

async fn connection(socket: &mut WebSocket, app: App, id: String) -> anyhow::Result<()> {
    use anyhow::{bail, Context};
    let nonce = uuid::Uuid::new_v4().to_string();
    send(
        socket,
        &DirectEvent::Challenge {
            device_id: id.clone(),
            nonce: nonce.clone(),
        },
    )
    .await?;
    let frame = tokio::time::timeout(Duration::from_secs(10), socket.recv())
        .await?
        .context("registration missing")??;
    let WsMessage::Text(text) = frame else {
        bail!("registration missing");
    };
    let DirectRequest::Register {
        alias,
        certificate,
        signature,
    } = serde_json::from_str(&text)?
    else {
        bail!("registration missing");
    };
    let device = verify(id.clone(), &nonce, alias, &certificate, &signature)?;
    let (tx, mut rx) = mpsc::channel(32);
    {
        let mut devices = app.0.direct.0.lock().unwrap();
        if devices.contains_key(&id) {
            bail!("device already connected");
        }
        devices.insert(id.clone(), (device.clone(), tx));
    }
    let _guard = Registration(app.clone(), id.clone());
    let mut tick = tokio::time::interval(Duration::from_secs(2));
    let mut heartbeat = tokio::time::interval(Duration::from_secs(20));
    let mut last_seen = tokio::time::Instant::now();
    loop {
        tokio::select! {
            incoming = socket.recv() => {
                let Some(Ok(frame)) = incoming else { break; };
                last_seen = tokio::time::Instant::now();
                if let WsMessage::Close(_) = frame { break; }
                if let WsMessage::Text(text) = frame {
                    let DirectRequest::Signal { to, session, offer, sdp } = serde_json::from_str(&text)? else { bail!("duplicate registration"); };
                    if to == id || uuid::Uuid::parse_str(&session).is_err() || !(sdp == HTTPS_RELAY || direct_sdp(&sdp)) {
                        bail!("invalid session description");
                    }
                    if sdp == HTTPS_RELAY {
                        let allowed = {
                            let mut pairs = app.0.direct.1.lock().unwrap();
                            let now = std::time::Instant::now();
                            pairs.retain(|_, pair| pair.expires > now);
                            if offer && pairs.len() < 64 && !pairs.contains_key(&session) {
                                pairs.insert(session.clone(), Pair {
                                    from: id.clone(), to: to.clone(), expires: now + Duration::from_secs(25), waiting: None,
                                });
                                true
                            } else {
                                !offer && pairs.get(&session).is_some_and(|pair| pair.from == to && pair.to == id)
                            }
                        };
                        if !allowed { send(socket, &DirectEvent::Rejected { session }).await?; continue; }
                    }
                    let sent = app.0.direct.0.lock().unwrap().get(&to).is_some_and(|(_, tx)| tx.try_send(
                        DirectEvent::Signal { from: device.clone(), session: session.clone(), offer, sdp }
                    ).is_ok());
                    if !sent { send(socket, &DirectEvent::Rejected { session }).await?; }
                }
            }
            Some(event) = rx.recv() => send(socket, &event).await?,
            _ = tick.tick() => {
                let devices = app.0.direct.0.lock().unwrap().values().map(|(device, _)| device.clone()).collect();
                send(socket, &DirectEvent::Devices { devices }).await?;
            }
            _ = heartbeat.tick() => {
                if last_seen.elapsed() > Duration::from_secs(60) { break; }
                tokio::time::timeout(Duration::from_secs(5), socket.send(WsMessage::Ping(Vec::new().into()))).await??;
            }
        }
    }
    Ok(())
}
