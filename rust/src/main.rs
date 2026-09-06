//! antify-rs CLI：LocalSend v2 节点（发现 / 接收 / 发送 / 面板）
use antify_rs::{client, config::DEFAULT_PORT, discovery, server, state, state::Device};
use clap::{Parser, Subcommand};
use std::net::IpAddr;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Parser)]
#[command(
    name = "antify-rs",
    version,
    about = "AntifyBot · LocalSend v2 节点 —— 与 LocalSend 官方 App 互通",
    disable_help_subcommand = true
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

/// CLI `serve` 的本机面板默认端口（与 Tauri 壳 UI_PORT_BASE 一致）
const UI_PORT_DEFAULT: u16 = 53318;

#[derive(Subcommand)]
enum Cmd {
    /// 启动常驻节点：UDP 多播发现 + HTTPS 接收服务 + Web 面板
    Serve {
        /// HTTPS 端口（默认 53317，LocalSend 标准）
        #[arg(long)]
        port: Option<u16>,
        /// 设备别名（默认 主机名 🐜）
        #[arg(long)]
        alias: Option<String>,
        /// 接收文件的保存目录（默认 ~/Downloads/AntifyBot）
        #[arg(long)]
        dir: Option<PathBuf>,
        /// 关闭多播发现（仅手动添加设备）
        #[arg(long)]
        no_multicast: bool,
        /// 多播端口（默认与 HTTPS 端口同为 53317）
        #[arg(long)]
        multicast_port: Option<u16>,
        /// 本机面板 HTTP 端口（默认 53318，被占自动顺延；仅绑 127.0.0.1）
        #[arg(long, default_value_t = UI_PORT_DEFAULT)]
        ui_port: u16,
    },
    /// 监听多播几秒，列出当前网段里的 LocalSend 设备
    Discover {
        /// 监听时长（秒）
        #[arg(long, default_value_t = 5)]
        secs: u64,
        /// 自身别名（用于公告，让对方也回 register）
        #[arg(long)]
        alias: Option<String>,
    },
    /// 向某个设备发送文件或一段文本
    Send {
        /// 要发送的文件（可多个；与 --text 二选一）
        files: Vec<PathBuf>,
        /// 要发送的文本
        #[arg(long)]
        text: Option<String>,
        /// 目标：别名子串或指纹前缀（配合自动发现）
        #[arg(long, group = "target")]
        to: Option<String>,
        /// 目标：直接指定 host:port（跳过发现）
        #[arg(long, group = "target")]
        host: Option<String>,
        /// 等待发现目标的最长秒数
        #[arg(long, default_value_t = 8)]
        wait: u64,
        #[arg(long)]
        alias: Option<String>,
        #[arg(long, default_value_t = DEFAULT_PORT)]
        port: u16,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    antify_rs::install_crypto_provider();
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Serve {
            port,
            alias,
            dir,
            no_multicast,
            multicast_port,
            ui_port,
        } => serve(port, alias, dir, !no_multicast, multicast_port, ui_port).await,
        Cmd::Discover { secs, alias } => discover(secs, alias).await,
        Cmd::Send {
            files,
            text,
            to,
            host,
            wait,
            alias,
            port,
        } => send(files, text, to, host, wait, alias, port).await,
    }
}

async fn serve(
    port: Option<u16>,
    alias: Option<String>,
    dir: Option<PathBuf>,
    multicast: bool,
    multicast_port: Option<u16>,
    ui_port: u16,
) -> anyhow::Result<()> {
    let port = port.unwrap_or(DEFAULT_PORT);
    let state = antify_rs::start_node(port, alias, dir, multicast, multicast_port).await?;
    let me = &state.identity;

    // 本机面板：默认常开（仅绑 127.0.0.1；协议端口 0.0.0.0 上不再伺服面板，避免局域网可调）
    let ui_port = server::pick_free_port(ui_port);
    {
        let st = state.clone();
        tokio::spawn(async move {
            if let Err(e) = server::serve_ui_http(st, ui_port).await {
                eprintln!("[面板] HTTP 服务退出: {e:#}");
            }
        });
    }

    println!("┌────────────────────────────────────────────");
    println!("│ 🐜 AntifyBot 节点已启动");
    println!("│ 别名     : {}", me.alias);
    println!("│ 指纹     : {}", me.fingerprint);
    println!("│ 面板     : http://127.0.0.1:{ui_port}");
    println!("│          （仅本机可访问；协议端口 {} 只收发文件）", me.port);
    println!("│ 保存目录 : {}", me.download_dir.display());
    println!("│ 多播发现 : {}", if multicast { "开启（224.0.0.167:53317）" } else { "关闭" });
    println!("└────────────────────────────────────────────");

    // 原生文件选择器桥：CLI 无 NSApplication，macOS 上 rfd 只能在主线程调。
    // 面板的 /api/ui/pick 经 mpsc 转到这里；主线程同步轮询通道 + 退出标志
    // （阻塞主线程不影响 worker 线程上的发现/收发/面板任务）
    let (pick_tx, pick_rx) = std::sync::mpsc::channel::<state::PickJob>();
    *state.picker.lock().await = state::Picker::Bridge(pick_tx);
    let (quit_tx, quit_rx) = std::sync::mpsc::channel::<()>();
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        let _ = quit_tx.send(());
    });
    loop {
        match pick_rx.recv_timeout(Duration::from_millis(300)) {
            Ok(job) => {
                let state::PickJob { folder, reply } = job;
                // 选择框期间主线程阻塞是预期行为；panic（异常环境）降级为「取消」
                let picked = std::panic::catch_unwind(move || {
                    if folder {
                        rfd::FileDialog::new().pick_folder().map(|p| vec![p])
                    } else {
                        rfd::FileDialog::new().pick_files()
                    }
                })
                .unwrap_or(None);
                let _ = reply.send(picked);
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if quit_rx.try_recv().is_ok() {
                    break;
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    println!("再见 👋");
    Ok(())
}

/// 监听 secs 秒多播，打印发现的设备。也公告自己，让对方回 register（信息更全）。
async fn discover(secs: u64, alias: Option<String>) -> anyhow::Result<()> {
    // 一次性命令：只读身份，不覆盖常驻节点已保存的别名
    let identity = antify_rs::config::Identity::load_opts(alias, DEFAULT_PORT, None, false)?;
    let state = server::new_state(identity)?;
    let client = server::http_client();

    let st = state.clone();
    let task = tokio::spawn(async move { discovery::start(st, client, None).await });
    println!("🐝 监听 LocalSend 多播（{secs} 秒）……");
    tokio::time::sleep(Duration::from_secs(secs)).await;
    task.abort();

    let devices = state.devices.lock().await;
    if devices.is_empty() {
        println!("（没有发现任何设备）");
        return Ok(());
    }
    println!("{:<24} {:<8} {:<22} {}", "别名", "版本", "地址", "指纹");
    for d in devices.values() {
        println!(
            "{:<24} {:<8} {:<22} {}…",
            d.alias,
            d.version,
            format!("{}:{}", d.ip, d.port),
            &d.fingerprint[..12.min(d.fingerprint.len())]
        );
    }
    Ok(())
}

async fn send(
    files: Vec<PathBuf>,
    text: Option<String>,
    to: Option<String>,
    host: Option<String>,
    wait: u64,
    alias: Option<String>,
    port: u16,
) -> anyhow::Result<()> {
    // 目标解析：--host 直连，否则等发现匹配 --to
    let target = match (&host, &to) {
        (Some(h), _) => {
            let (ip, tport) = parse_host(h, port)?;
            probe_device(ip, tport).await?
        }
        (None, Some(name)) => {
            let identity = antify_rs::config::Identity::load_opts(alias.clone(), port, None, false)?;
            let state = server::new_state(identity)?;
            let client = server::http_client();
            let st = state.clone();
            let cl = client.clone();
            tokio::spawn(async move { discovery::start(st, cl, None).await });
            println!("🔎 正在发现设备，匹配「{name}」（最多 {wait} 秒）……");
            let deadline = tokio::time::Instant::now() + Duration::from_secs(wait);
            let mut found: Option<Device> = None;
            while tokio::time::Instant::now() < deadline {
                tokio::time::sleep(Duration::from_millis(400)).await;
                let devices = state.devices.lock().await;
                found = devices
                    .values()
                    .find(|d| {
                        let name_lower = name.to_lowercase();
                        d.alias.to_lowercase().contains(&name_lower)
                            || d.fingerprint.to_lowercase().starts_with(&name_lower)
                            || d.fingerprint.to_lowercase() == name_lower
                    })
                    .cloned();
                if found.is_some() {
                    break;
                }
            }
            match found {
                Some(d) => d,
                None => anyhow::bail!("没找到匹配「{name}」的设备；用 `antify-rs discover` 先看看谁在线"),
            }
        }
        (None, None) => anyhow::bail!("需要 --to <别名|指纹> 或 --host <ip:port> 指定目标"),
    };

    println!(
        "→ 目标：{} （{}:{}，指纹 {}…）",
        target.alias,
        target.ip,
        target.port,
        &target.fingerprint[..12.min(target.fingerprint.len())]
    );

    let mut items = Vec::new();
    if let Some(t) = text {
        items.push(client::item_from_text(t));
    }
    for p in files {
        items.push(client::item_from_path(p.clone()).await?);
    }
    if items.is_empty() {
        anyhow::bail!("没有内容可发：给位置参数传文件，或 --text 传文本");
    }
    let identity = antify_rs::config::Identity::load_opts(alias, port, None, false)?;
    let state = server::new_state(identity)?;
    client::send(&state, &server::http_client(), &target, items).await
}

fn parse_host(h: &str, default_port: u16) -> anyhow::Result<(IpAddr, u16)> {
    let (ip_s, port_s) = match h.rsplit_once(':') {
        Some((ip, p)) => (ip.to_string(), p.to_string()),
        None => (h.to_string(), default_port.to_string()),
    };
    let ip: IpAddr = ip_s
        .trim_matches(['[', ']'])
        .parse()
        .map_err(|_| anyhow::anyhow!("IP 格式不对：{ip_s}"))?;
    Ok((
        ip,
        port_s
            .parse()
            .map_err(|_| anyhow::anyhow!("端口格式不对：{port_s}"))?,
    ))
}

/// GET /info 拿对方身份（--host 直连时）
async fn probe_device(ip: IpAddr, port: u16) -> anyhow::Result<Device> {
    let url = format!("https://{ip}:{port}/api/localsend/v2/info");
    let r = server::http_client()
        .get(&url)
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("连不上 {url}：{e}"))?;
    if !r.status().is_success() {
        anyhow::bail!("对方 /info 返回 {}", r.status());
    }
    let v: serde_json::Value = r.json().await?;
    Ok(Device {
        fingerprint: v
            .get("fingerprint")
            .and_then(|x| x.as_str())
            .unwrap_or_default()
            .to_string(),
        alias: v
            .get("alias")
            .and_then(|x| x.as_str())
            .unwrap_or("未知")
            .to_string(),
        ip,
        port,
        https: true,
        device_model: v.get("deviceModel").and_then(|x| x.as_str()).map(str::to_string),
        device_type: v.get("deviceType").and_then(|x| x.as_str()).map(str::to_string),
        version: v
            .get("version")
            .and_then(|x| x.as_str())
            .unwrap_or("?")
            .to_string(),
        download: v.get("download").and_then(|x| x.as_bool()).unwrap_or(false),
        last_seen: state::now_ms(),
    })
}
