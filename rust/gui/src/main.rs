//! AntifyBot 桌面壳（Tauri v2）
//! 后台启动 LocalSend 节点（多播发现 + HTTPS 收发），窗口加载本机明文面板。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::webview::WebviewWindowBuilder;
use tauri::{Manager, WebviewUrl, WindowEvent};

/// 本机面板端口：53318 起，占用则依次后挪
const UI_PORT_BASE: u16 = 53318;

fn main() {
    antify_rs::install_crypto_provider();

    tauri::Builder::default()
        .on_window_event(|window, event| {
            // 关窗不退出：收进托盘，节点常驻继续收发；退出走托盘菜单
            if let WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
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
            let window = WebviewWindowBuilder::new(app, "main", WebviewUrl::External(url))
                .title("AntifyBot")
                .inner_size(900.0, 680.0)
                .min_inner_size(720.0, 540.0);
            // macOS 让网页工具栏延伸至标题栏，同时保留系统交通灯。
            // Windows 仍使用系统标题栏，避免额外维护窗口控制按钮。
            #[cfg(target_os = "macos")]
            let window = window
                .title_bar_style(tauri::TitleBarStyle::Overlay)
                .hidden_title(true)
                // 与网页工具栏（44px）的中线一致；x 与失焦灰色占位灯同步。
                .traffic_light_position(tauri::LogicalPosition::new(8.0, 21.0));
            window.build()?;

            // 5) 系统托盘：左键点图标切换窗口显隐，菜单提供「显示面板 / 退出」
            let show = MenuItem::with_id(app, "show", "显示面板", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "退出 AntifyBot", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &quit])?;
            TrayIconBuilder::with_id("main-tray")
                .icon(app.default_window_icon().cloned().ok_or("缺少应用图标")?)
                .tooltip("AntifyBot 局域网快传")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => show_main_window(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if matches!(
                        event,
                        TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            ..
                        }
                    ) {
                        toggle_main_window(tray.app_handle());
                    }
                })
                .build(app)?;
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("AntifyBot 运行失败")
        .run(handle_run_event);
}

/// macOS：窗口收进托盘后，点 Dock 图标重新唤起
#[cfg(target_os = "macos")]
fn handle_run_event(app: &tauri::AppHandle, event: tauri::RunEvent) {
    if let tauri::RunEvent::Reopen {
        has_visible_windows,
        ..
    } = event
    {
        if !has_visible_windows {
            show_main_window(app);
        }
    }
}

/// Windows：无 Dock 重开事件，无需处理
#[cfg(not(target_os = "macos"))]
fn handle_run_event(_app: &tauri::AppHandle, _event: tauri::RunEvent) {}

/// 唤起主窗口（显示 + 聚焦）
fn show_main_window(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.set_focus();
    }
}

/// 托盘左键切换：窗口可见且聚焦 → 收起，否则唤起
fn toggle_main_window(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        if w.is_visible().unwrap_or(false) && w.is_focused().unwrap_or(false) {
            let _ = w.hide();
        } else {
            let _ = w.show();
            let _ = w.set_focus();
        }
    }
}

fn bind_ui_port(base: u16) -> Option<(u16, std::net::TcpListener)> {
    (base..base + 20).find_map(|p| {
        std::net::TcpListener::bind(("127.0.0.1", p))
            .ok()
            .map(|l| (p, l))
    })
}
