//! antify-rs 库入口：LocalSend v2 节点（供 CLI 与 Tauri 壳共用）
pub mod client;
pub mod config;
pub mod discovery;
pub mod server;
pub mod state;
pub mod ui;

use state::Shared;
use std::path::PathBuf;

/// 选定 rustls CryptoProvider（进程内必须先于任何 TLS 使用调用一次）
pub fn install_crypto_provider() {
    rustls::crypto::ring::default_provider().install_default().ok();
}

/// 启动一个常驻节点（发现 + HTTPS 服务），返回共享状态。
/// GUI 与 `serve` 子命令共用。
pub async fn start_node(
    port: u16,
    alias: Option<String>,
    dir: Option<PathBuf>,
    multicast: bool,
    multicast_port: Option<u16>,
) -> anyhow::Result<Shared> {
    // 端口被占（如同机跑着官方 LocalSend）时自动顺延，公告携带真实端口
    let actual_port = server::pick_free_port(port);
    let identity = config::Identity::load(alias, actual_port, dir)?;
    let state = server::new_state(identity);
    if actual_port != port {
        state
            .log_event(format!("端口 {port} 被占用，HTTPS 已改用 {actual_port}"))
            .await;
    }

    if multicast {
        let st = state.clone();
        let client = server::http_client();
        tokio::spawn(async move {
            if let Err(e) = discovery::start(st, client, multicast_port).await {
                eprintln!("[发现] 启动失败: {e:#}（仍可手动添加设备）");
            }
        });
    }

    let st = state.clone();
    tokio::spawn(async move {
        if let Err(e) = server::serve(st).await {
            eprintln!("[HTTPS] 服务退出: {e:#}");
        }
    });

    Ok(state)
}
