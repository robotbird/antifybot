use anyhow::{ensure, Context, Result};
use serde::{de::DeserializeOwned, Serialize};
use std::{sync::Arc, time::Duration};
use tokio::sync::mpsc;
use webrtc::{
    api::{setting_engine::SettingEngine, APIBuilder},
    data_channel::RTCDataChannel,
    ice_transport::ice_server::RTCIceServer,
    peer_connection::{
        configuration::RTCConfiguration, sdp::session_description::RTCSessionDescription,
        RTCPeerConnection,
    },
};
use webrtc_data::data_channel::DataChannel;

pub(super) const PACKET: usize = 16 * 1024;
const CONTROL_LIMIT: usize = 256 * 1024;
const IO_TIMEOUT: Duration = Duration::from_secs(60);

pub(super) struct Connection(pub Arc<RTCPeerConnection>);

impl Connection {
    pub async fn new(stun: Option<&str>) -> Result<Self> {
        let mut settings = SettingEngine::default();
        settings.detach_data_channels();
        #[cfg(target_os = "macos")]
        if let Ok(interfaces) = if_addrs::get_if_addrs() {
            let proxy_interfaces: Vec<_> = interfaces
                .into_iter()
                .filter(|interface| {
                    interface.name.starts_with("utun")
                        && matches!(interface.ip(), std::net::IpAddr::V4(ip)
                            if ip.octets()[0] == 198 && matches!(ip.octets()[1], 18 | 19))
                })
                .map(|interface| interface.name)
                .collect();
            if !proxy_interfaces.is_empty() {
                settings.set_interface_filter(Box::new(move |name| {
                    !proxy_interfaces.iter().any(|interface| interface == name)
                }));
            }
        }
        let config = RTCConfiguration {
            ice_servers: stun
                .into_iter()
                .map(|url| RTCIceServer {
                    urls: vec![url.to_owned()],
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };
        let connection = APIBuilder::new()
            .with_setting_engine(settings)
            .build()
            .new_peer_connection(config)
            .await?;
        Ok(Self(Arc::new(connection)))
    }

    pub async fn description(&self, offer: bool) -> Result<String> {
        let description = if offer {
            self.0.create_offer(None).await?
        } else {
            self.0.create_answer(None).await?
        };
        let mut gathering = self.0.gathering_complete_promise().await;
        self.0.set_local_description(description).await?;
        let _ = tokio::time::timeout(Duration::from_secs(8), gathering.recv()).await;
        let sdp = self
            .0
            .local_description()
            .await
            .context("缺少本地连接信息")?
            .sdp;
        ensure!(antify_protocol::direct_sdp(&sdp), "只允许直连候选");
        Ok(sdp)
    }

    pub async fn path(&self) -> Option<String> {
        let dtls = self.0.sctp().transport();
        let pair = dtls.ice_transport().get_selected_candidate_pair().await?;
        let interfaces = if_addrs::get_if_addrs().unwrap_or_default();
        let interface = |address: std::net::IpAddr| {
            interfaces
                .iter()
                .find(|candidate| candidate.ip() == address)
                .map(|candidate| candidate.name.as_str())
        };
        let local_interface = [&pair.local.related_address, &pair.local.address]
            .into_iter()
            .filter_map(|address| address.parse().ok())
            .find_map(interface)
            .unwrap_or("?");
        // UDP connect asks the OS to select a local source address without sending a datagram.
        let default_interface = pair
            .remote
            .address
            .parse::<std::net::IpAddr>()
            .ok()
            .and_then(|remote| {
                let bind = if remote.is_ipv4() {
                    "0.0.0.0:0"
                } else {
                    "[::]:0"
                };
                let socket = std::net::UdpSocket::bind(bind).ok()?;
                socket
                    .connect(std::net::SocketAddr::new(remote, pair.remote.port))
                    .ok()?;
                interface(socket.local_addr().ok()?.ip())
            })
            .unwrap_or("?");
        Some(format!(
            "{} {} → {} {}，候选网卡 {local_interface}，系统选路网卡 {default_interface}",
            pair.local.protocol, pair.local.typ, pair.remote.protocol, pair.remote.typ
        ))
    }

    pub async fn remote(&self, offer: bool, sdp: String) -> Result<()> {
        ensure!(antify_protocol::direct_sdp(&sdp), "只允许直连候选");
        let sdp = if offer {
            RTCSessionDescription::offer(sdp)?
        } else {
            RTCSessionDescription::answer(sdp)?
        };
        self.0.set_remote_description(sdp).await?;
        Ok(())
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        let peer = self.0.clone();
        tokio::spawn(async move {
            let _ = peer.close().await;
        });
    }
}

pub(super) fn detach(channel: Arc<RTCDataChannel>, opened: mpsc::Sender<Wire>) {
    let weak = Arc::downgrade(&channel);
    channel.on_open(Box::new(move || {
        Box::pin(async move {
            if let Some(channel) = weak.upgrade() {
                if let Ok(channel) = channel.detach().await {
                    let _ = opened.try_send(Wire::Direct(channel));
                }
            }
        })
    }));
}

#[derive(Debug)]
pub(super) struct Interrupted;
impl std::fmt::Display for Interrupted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("点对点连接已中断")
    }
}
impl std::error::Error for Interrupted {}

type RelaySocket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;
pub(super) enum Wire {
    Direct(Arc<DataChannel>),
    Relay {
        writer: tokio::sync::Mutex<
            futures_util::stream::SplitSink<RelaySocket, tokio_tungstenite::tungstenite::Message>,
        >,
        reader: tokio::sync::Mutex<futures_util::stream::SplitStream<RelaySocket>>,
    },
}

impl Wire {
    pub fn relay(socket: RelaySocket) -> Self {
        use futures_util::StreamExt;
        let (writer, reader) = socket.split();
        Self::Relay {
            writer: tokio::sync::Mutex::new(writer),
            reader: tokio::sync::Mutex::new(reader),
        }
    }

    pub fn is_relay(&self) -> bool {
        matches!(self, Self::Relay { .. })
    }

    pub async fn write(&self, packet: &[u8]) -> Result<()> {
        ensure!(
            !packet.is_empty() && packet.len() <= PACKET,
            "数据包大小无效"
        );
        let bytes = axum::body::Bytes::copy_from_slice(packet);
        match self {
            Self::Direct(channel) => {
                let n = tokio::time::timeout(IO_TIMEOUT, channel.write(&bytes))
                    .await
                    .map_err(|_| Interrupted)?
                    .map_err(|_| Interrupted)?;
                ensure!(n == packet.len(), "点对点写入中断");
            }
            Self::Relay { writer, .. } => {
                use futures_util::SinkExt;
                tokio::time::timeout(IO_TIMEOUT, async {
                    writer
                        .lock()
                        .await
                        .send(tokio_tungstenite::tungstenite::Message::Binary(bytes))
                        .await
                })
                .await
                .map_err(|_| Interrupted)?
                .map_err(|_| Interrupted)?;
            }
        }
        Ok(())
    }

    pub async fn read(&self, packet: &mut [u8]) -> Result<usize> {
        let n = match self {
            Self::Direct(channel) => tokio::time::timeout(IO_TIMEOUT, channel.read(packet))
                .await
                .map_err(|_| Interrupted)?
                .map_err(|_| Interrupted)?,
            Self::Relay { reader, .. } => {
                use futures_util::StreamExt;
                use tokio_tungstenite::tungstenite::Message;
                let data = tokio::time::timeout(IO_TIMEOUT, async {
                    let mut reader = reader.lock().await;
                    loop {
                        match reader.next().await {
                            Some(Ok(Message::Binary(bytes))) => return Ok(bytes),
                            Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
                            _ => return Err(Interrupted),
                        }
                    }
                })
                .await
                .map_err(|_| Interrupted)??;
                ensure!(data.len() <= packet.len(), "中转数据包大小无效");
                packet[..data.len()].copy_from_slice(&data);
                data.len()
            }
        };
        if n == 0 {
            return Err(Interrupted.into());
        }
        ensure!(n <= packet.len(), "数据包大小无效");
        Ok(n)
    }

    pub async fn send<T: Serialize>(&self, value: &T) -> Result<()> {
        let bytes = serde_json::to_vec(value)?;
        ensure!(bytes.len() <= CONTROL_LIMIT, "控制消息过长");
        self.write(&(bytes.len() as u32).to_be_bytes()).await?;
        for packet in bytes.chunks(PACKET) {
            self.write(packet).await?;
        }
        Ok(())
    }

    pub async fn recv<T: DeserializeOwned>(&self) -> Result<T> {
        let mut packet = vec![0; PACKET];
        ensure!(self.read(&mut packet).await? == 4, "控制消息头无效");
        let len = u32::from_be_bytes(packet[..4].try_into()?) as usize;
        ensure!(len > 0 && len <= CONTROL_LIMIT, "控制消息长度无效");
        let mut bytes = Vec::with_capacity(len);
        while bytes.len() < len {
            let n = self.read(&mut packet).await?;
            ensure!(n <= len - bytes.len(), "控制消息长度不匹配");
            bytes.extend_from_slice(&packet[..n]);
        }
        Ok(serde_json::from_slice(&bytes)?)
    }
}
