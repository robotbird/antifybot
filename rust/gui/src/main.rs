//! AntifyBot 桌面壳（Tauri v2）
//! 后台启动 LocalSend 节点（多播发现 + HTTPS 收发），窗口加载本机明文面板。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use tauri::webview::WebviewWindowBuilder;
use tauri::WebviewUrl;

/// 本机面板端口：53318 起，占用则依次后挪
const UI_PORT_BASE: u16 = 53318;

fn main() {
    antify_rs::install_crypto_provider();

    tauri::Builder::default()
        .setup(|app| {
            // 1) 先占住面板端口（同步，保证窗口打开时服务已就绪）
            let (ui_port, listener) = bind_ui_port(UI_PORT_BASE)
                .ok_or("没有可用面板端口（53318 起 20 个都被占了）")?;

            // 2) 节点：多播发现 + HTTPS(LocalSend 标准 53317)
            let state = tauri::async_runtime::block_on(antify_rs::start_node(
                53317, None, None, true, None,
            ))?;

            // 3) 面板 HTTP 服务（接管上面占住的 listener）
            tauri::async_runtime::spawn(async move {
                let router =
                    antify_rs::server::build_panel_router(state).into_make_service_with_connect_info::<std::net::SocketAddr>();
                if let Err(e) =
                    axum_server::from_tcp(listener).serve(router).await
                {
                    eprintln!("[面板] HTTP 服务退出: {e:#}");
                }
            });
            eprintln!("[面板] http://127.0.0.1:{ui_port}");

            // 4) 窗口
            let url: tauri::Url = format!("http://127.0.0.1:{ui_port}/")
                .parse()
                .map_err(|e| format!("面板 URL 解析失败: {e}"))?;
            WebviewWindowBuilder::new(app, "main", WebviewUrl::External(url))
                .title("AntifyBot")
                .inner_size(900.0, 680.0)
                .min_inner_size(720.0, 540.0)
                .build()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("AntifyBot 运行失败");
}

fn bind_ui_port(base: u16) -> Option<(u16, std::net::TcpListener)> {
    (base..base + 20).find_map(|p| {
        std::net::TcpListener::bind(("127.0.0.1", p))
            .ok()
            .map(|l| (p, l))
    })
}
