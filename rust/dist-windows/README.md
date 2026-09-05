# 🐜 AntifyBot Windows 版

本目录是 Windows x86-64 可执行文件（从 macOS 交叉编译产出，已在依赖层面验证编译/链接通过）。

## 两个文件

| 文件 | 用途 |
| --- | --- |
| `antify-gui.exe` | 桌面应用（Tauri 窗口，双击即用；需 WebView2 运行时，Win11 自带，Win10 大多已随 Edge 安装） |
| `antify-rs.exe` | 无头 CLI 节点（命令行使用） |

## 快速开始

双击 `antify-gui.exe`，或命令行：

```powershell
.\antify-rs.exe serve                        # 常驻节点
.\antify-rs.exe discover                     # 扫描网段里的 LocalSend 设备
.\antify-rs.exe send photo.jpg --to 手机      # 发文件（自动等待发现匹配）
.\antify-rs.exe send --text "hi" --host 192.168.1.5:53317   # 直连发送
```

## 注意事项

- **首次运行 Windows 会弹防火墙授权**，请点「允许」（专用 + 公用网络均可勾选），否则收不到局域网设备。
- 与官方 LocalSend App 同机运行时：HTTPS 端口 53317 被占会自动顺延（面板与公告会显示/携带真实端口），多播监听退化为只公告。
- 未签名 exe：SmartScreen 可能提示「更多信息 → 仍要运行」。
- 接收目录默认 `下载\AntifyBot`；配置与证书在 `%USERPROFILE%\.antifybot-rs\`。
- 自行构建：装 Rust（MSVC 工具链）后在 `rust/` 目录 `cargo build --release`，或用 `build-win.ps1`。
