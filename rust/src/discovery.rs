//! UDP 多播发现：224.0.0.167:53317
//! 收到他人公告 → 记入设备表 → 回一个 HTTP register（对方响应里带回它的最新信息）
//! 自己启动时发公告爆发（100/500/2000ms × 3 包），之后每 2 分钟重发保持可见
//!
//! 接口热变化：Wi-Fi 漫游 / 热点重编号 / 虚拟机网桥启停都会让启动时枚举的地址失效
//! （入组与出口接口都钉死在旧地址上，节点从此失聪）。因此每 15s 重枚举一次，
//! 集合有变就热替换收发 socket —— 接收 socket 重建时靠 REUSEPORT 无缝共存，无收包空窗。
use crate::config::{MULTICAST_GROUP, DEFAULT_PORT};
use crate::state::{now_ms, Device, Shared};
use anyhow::Result;
use serde::Deserialize;
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::sync::{RwLock, watch};

/// 收到的多播消息（v2，宽松解析：老版本可缺 deviceModel/download 等）
#[derive(Debug, Deserialize)]
struct AnnounceIn {
    #[serde(default)]
    #[allow(dead_code)] // v2.1 之后 announce 只是遗留标志，收到即视为"在网"
    announce: bool,
    alias: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    device_model: Option<String>,
    #[serde(default)]
    device_type: Option<String>,
    fingerprint: String,
    port: u16,
    #[serde(default = "default_protocol")]
    protocol: String,
    #[serde(default)]
    download: bool,
}

fn default_protocol() -> String {
    "https".to_string()
}

/// 启动发现循环（含公告）。`multicast_port` 独立于 HTTP 端口，默认 53317。
pub async fn start(state: Shared, client: reqwest::Client, multicast_port: Option<u16>) -> Result<()> {
    let mport = multicast_port.unwrap_or(DEFAULT_PORT);
    let group: std::net::SocketAddr = (MULTICAST_GROUP, mport).into();

    // 多播走遍所有网段：NAT 内的虚拟机网桥（bridge100 / vmnet8 / 10.211.55.x）
    // 与物理网卡互不可达（多播 TTL=1 不穿 NAT），但都直连本机 —— 每个接口各自收发即可互通
    let ifaces = mcast_interfaces();
    let send_socks = Arc::new(RwLock::new(build_send_socks(&ifaces)));
    let (recv_tx, recv_rx) = watch::channel(bind_recv(&state, mport, &ifaces).await);

    // 接口监视器：集合变化 → 热替换收发 socket（含降级模式的自动恢复）
    tokio::spawn(watch_interfaces(
        state.clone(),
        mport,
        send_socks.clone(),
        recv_tx,
        ifaces,
    ));

    let my_fp = state.identity.fingerprint.clone();
    // 收到陌生公告时的"回敬公告"限流：避免多设备同时在线时风暴
    let last_reannounce = Arc::new(std::sync::atomic::AtomicU64::new(0));

    // 公告爆发 + 周期重发
    tokio::spawn({
        let state = state.clone();
        let socks = send_socks.clone();
        async move {
            loop {
                for delay in [100u64, 500, 2000] {
                    tokio::time::sleep(Duration::from_millis(delay)).await;
                    announce_once(&state, &socks.read().await, group).await;
                }
                tokio::time::sleep(Duration::from_secs(120)).await;
            }
        }
    });

    // 接收循环：socket 可被监视器热替换；None = 端口暂被占用（只公告），等它换回新的
    let mut recv_rx = recv_rx;
    let mut cur = recv_rx.borrow_and_update().clone();
    let mut buf = vec![0u8; 65536];
    loop {
        let sock = match cur.clone() {
            Some(s) => s,
            None => {
                if recv_rx.changed().await.is_err() {
                    return Ok(()); // 监视器任务没了（不应发生）
                }
                cur = recv_rx.borrow_and_update().clone();
                continue;
            }
        };
        let (n, src) = tokio::select! {
            r = sock.recv_from(&mut buf) => match r {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("[发现] 接收失败: {e}，1s 后重试");
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    continue;
                }
            },
            _ = recv_rx.changed() => {
                cur = recv_rx.borrow_and_update().clone();
                continue;
            }
        };
        let msg: AnnounceIn = match serde_json::from_slice(&buf[..n]) {
            Ok(m) => m,
            Err(_) => continue, // 非 LocalSend 报文，忽略
        };
        if msg.fingerprint == my_fp {
            continue; // 多播回环：自己的公告
        }
        let device = Device {
            fingerprint: msg.fingerprint.clone(),
            alias: msg.alias.clone(),
            ip: src.ip(),
            port: msg.port,
            https: msg.protocol.eq_ignore_ascii_case("https"),
            device_model: msg.device_model.clone(),
            device_type: msg.device_type.clone(),
            version: msg.version.clone(),
            download: msg.download,
            last_seen: now_ms(),
        };
        state
            .log_event(format!(
                "发现设备「{}」 {}://{}:{}",
                msg.alias,
                if device.https { "https" } else { "http" },
                src.ip(),
                msg.port
            ))
            .await;
        upsert(state.clone(), device.clone()).await;

        // v2 礼仪：向公告方回 register（响应带回对方最新信息）
        let st = state.clone();
        let cl = client.clone();
        tokio::spawn(async move {
            register_back(st, cl, device).await;
        });

        // 对方刚上线大概率还没听过我们 —— 立即回敬一次公告（3 秒限流），
        // 让双向发现 ~1 秒完成，而不是等下一个 120 秒周期
        let now = crate::state::now_ms();
        let last = last_reannounce.load(std::sync::atomic::Ordering::Relaxed);
        if now.saturating_sub(last) > 3_000
            && last_reannounce
                .compare_exchange(last, now, std::sync::atomic::Ordering::Relaxed, std::sync::atomic::Ordering::Relaxed)
                .is_ok()
        {
            let st = state.clone();
            let socks = send_socks.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(150)).await;
                announce_once(&st, &socks.read().await, group).await;
            });
        }
    }
}

/// 周期重枚举接口；集合变化、或接收 socket 缺位（端口被独占的降级模式）时重建。
/// 端口释放、Wi-Fi 漫游、虚拟机网桥启停都会在这里被自动接住。
async fn watch_interfaces(
    state: Shared,
    mport: u16,
    send_socks: Arc<RwLock<Vec<Arc<UdpSocket>>>>,
    recv_tx: watch::Sender<Option<Arc<UdpSocket>>>,
    mut last: Vec<Ipv4Addr>,
) {
    let mut have_recv = recv_tx.borrow().is_some();
    loop {
        tokio::time::sleep(Duration::from_secs(15)).await;
        let ifaces = mcast_interfaces();
        if ifaces == last && have_recv {
            continue;
        }
        if ifaces != last {
            eprintln!(
                "[发现] 网络接口变化：{last:?} → {ifaces:?}，重建多播收发"
            );
            *send_socks.write().await = build_send_socks(&ifaces);
        }
        let new_recv = match bind_socket(mport, &ifaces) {
            Ok(s) => UdpSocket::from_std(s).ok().map(Arc::new),
            Err(e) => {
                if have_recv {
                    // 只在状态翻转时记事件，避免长期占用期间刷屏
                    state
                        .log_event(format!("UDP {mport} 被占用（{e}），暂只公告不监听，稍后自动重试"))
                        .await;
                }
                None
            }
        };
        if new_recv.is_some() && !have_recv {
            state.log_event("多播监听已恢复").await;
        }
        if ifaces != last {
            state
                .log_event(format!(
                    "网络接口变化，多播收发已切换到 {}",
                    if ifaces.is_empty() {
                        "系统默认接口".to_string()
                    } else {
                        ifaces.iter().map(|ip| ip.to_string()).collect::<Vec<_>>().join(", ")
                    }
                ))
                .await;
        }
        have_recv = new_recv.is_some();
        let _ = recv_tx.send(new_recv);
        last = ifaces;
    }
}

/// 每个接口一个出口 socket（不绑定端口，源端口由系统分配；
/// 接收方回礼走 TCP，用的是公告载荷里的 HTTPS 端口，与 UDP 源端口无关）
fn build_send_socks(ifaces: &[Ipv4Addr]) -> Vec<Arc<UdpSocket>> {
    let mut socks = Vec::new();
    for ip in ifaces {
        if let Ok(t) = UdpSocket::from_std(send_socket(Some(*ip))) {
            socks.push(Arc::new(t));
        }
    }
    if socks.is_empty() {
        // 兜底：交给系统默认接口
        if let Ok(t) = UdpSocket::from_std(send_socket(None)) {
            socks.push(Arc::new(t));
        }
    }
    socks
}

/// 绑定 UDP 多播接收 socket；被独占时返回 None（降级为只公告，watcher 稍后重试）。
async fn bind_recv(state: &Shared, mport: u16, ifaces: &[Ipv4Addr]) -> Option<Arc<UdpSocket>> {
    match bind_socket(mport, ifaces) {
        Ok(s) => UdpSocket::from_std(s).ok().map(Arc::new),
        Err(e) => {
            state
                .log_event(format!("UDP {mport} 被占用（{e}），暂只公告不监听，稍后自动重试"))
                .await;
            None
        }
    }
}

async fn announce_once(state: &Shared, socks: &[Arc<UdpSocket>], group: std::net::SocketAddr) {
    let payload = state.identity.announce_json(true).to_string();
    for sock in socks {
        if let Err(e) = sock.send_to(payload.as_bytes(), group).await {
            eprintln!("[发现] 公告发送失败（{}）: {e}", sock.local_addr().map(|a| a.to_string()).unwrap_or_default());
        }
    }
}

async fn upsert(state: Shared, device: Device) {
    state
        .devices
        .lock()
        .await
        .insert(device.fingerprint.clone(), device);
}

/// POST /api/localsend/v2/register —— 响应（若有）刷新对方信息
async fn register_back(state: Shared, client: reqwest::Client, device: Device) {
    let url = format!("{}/api/localsend/v2/register", device.base_url());
    let body = state.identity.register_json();
    let resp = client.post(&url).json(&body).timeout(Duration::from_secs(5)).send().await;
    match resp {
        Ok(r) if r.status().is_success() => {
            if let Ok(v) = r.json::<serde_json::Value>().await {
                let alias = v.get("alias").and_then(|x| x.as_str()).map(str::to_string);
                let mut devices = state.devices.lock().await;
                if let Some(d) = devices.get_mut(&device.fingerprint) {
                    if let Some(a) = alias {
                        d.alias = a;
                    }
                    if let Some(m) = v.get("deviceModel").and_then(|x| x.as_str()) {
                        d.device_model = Some(m.to_string());
                    }
                    if let Some(t) = v.get("deviceType").and_then(|x| x.as_str()) {
                        d.device_type = Some(t.to_string());
                    }
                    if let Some(dl) = v.get("download").and_then(|x| x.as_bool()) {
                        d.download = dl;
                    }
                    d.last_seen = now_ms();
                }
            }
        }
        Ok(r) => {
            state
                .log_event(format!("register 到「{}」返回 {}", device.alias, r.status()))
                .await;
        }
        Err(e) => {
            state
                .log_event(format!("register 到「{}」失败: {e}", device.alias))
                .await;
        }
    }
}

/// 枚举适合多播的本地 IPv4 接口（跳过回环与 utun/tun/tap 隧道）。
/// 返回空 = 交给系统默认接口。
fn mcast_interfaces() -> Vec<Ipv4Addr> {
    let mut out: Vec<Ipv4Addr> = Vec::new();
    if let Ok(list) = if_addrs::get_if_addrs() {
        for itf in list {
            let name = itf.name.to_lowercase();
            if name.starts_with("utun") || name.starts_with("tun") || name.starts_with("tap") {
                continue; // VPN/代理 TUN 隧道：多播进去只会被吞
            }
            if let std::net::IpAddr::V4(ip) = itf.ip() {
                if ip.is_loopback() || ip.is_link_local() {
                    continue;
                }
                if !out.contains(&ip) {
                    out.push(ip);
                }
            }
        }
    }
    out
}

/// 绑定 UDP 多播接收 socket（socket2 跨平台；复用选项必须在 bind 之前设置才生效）。
/// 在每个接口上分别入组：来自任一网段（含虚拟机网桥）的公告都能收到。
/// Windows 没有 SO_REUSEPORT，由 SO_REUSEADDR 覆盖同样语义（双方都设即可共存）。
fn bind_socket(port: u16, ifaces: &[Ipv4Addr]) -> Result<std::net::UdpSocket> {
    use socket2::{Domain, Protocol, Socket, Type};
    let sock = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    sock.set_reuse_address(true)?;
    #[cfg(unix)]
    sock.set_reuse_port(true)?;
    let addr: std::net::SocketAddr = ([0, 0, 0, 0], port).into();
    sock.bind(&addr.into())?;
    let mut joined = 0;
    for ip in ifaces {
        if sock.join_multicast_v4(&MULTICAST_GROUP, ip).is_ok() {
            joined += 1;
        }
    }
    if joined == 0 {
        sock.join_multicast_v4(&MULTICAST_GROUP, &Ipv4Addr::UNSPECIFIED)?;
    }
    sock.set_multicast_loop_v4(true)?;
    sock.set_multicast_ttl_v4(1)?; // 不跨路由器
    sock.set_nonblocking(true)?; // tokio 注册要求
    Ok(sock.into())
}

/// 指定出口接口的公告 socket
fn send_socket(iface: Option<Ipv4Addr>) -> std::net::UdpSocket {
    use socket2::{Domain, Protocol, Socket, Type};
    let sock = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP)).expect("创建 UDP socket");
    if let Some(ip) = iface {
        let _ = sock.set_multicast_if_v4(&ip);
    }
    let _ = sock.set_multicast_loop_v4(true);
    let _ = sock.set_multicast_ttl_v4(1);
    let _ = sock.set_nonblocking(true);
    sock.into()
}
