# 🐜 AntifyBot 局域网快传（Rust + Tauri）

> 同一内网下设备互传文件与消息，**与 [LocalSend](https://localsend.org) 官方 App 直接互通**。
> macOS / Windows 开箱即用，遵循 [localsend/protocol](https://github.com/localsend/protocol) v2
> （UDP 多播发现 + HTTPS 收发 + 自签证书指纹）。

**无需注册、无需登录、无需公网。** 手机装 LocalSend、电脑跑 AntifyBot，互相就能看见、互发文件。

---

## 快速开始

### macOS（Apple Silicon）

```bash
cd rust
cargo build --workspace --release
bash build-app.sh          # 产出 rust/AntifyBot.app（ad-hoc 签名，约 9.6MB）
open AntifyBot.app
```

或从 GitHub **Actions → build → Artifacts** 下载 `AntifyBot-macOS-arm64.zip`（每次推送 `rust/` 自动构建）。

### Windows（x64）

exe 不入库，两个获取渠道：

- **Actions Artifacts**：仓库 Actions 页面，每次构建可下载（含 `antify-gui.exe` + `antify-rs.exe`）
- **Releases**：推送 `v*` tag 自动构建发布

```powershell
.\antify-gui.exe            # 双击打开桌面窗口（需 WebView2 运行时，Win11 自带）
.\antify-rs.exe serve       # 或命令行节点
```

本机构建（装好 Rust MSVC 工具链）：

```powershell
cd rust
powershell -ExecutionPolicy Bypass -File build-win.ps1
```

### 无头 CLI 节点（两平台通用）

```bash
antify-rs serve                      # 常驻节点 + 本机面板 http://127.0.0.1:53318
antify-rs discover                   # 扫描当前网段的 LocalSend 设备
antify-rs send photo.jpg --to 手机    # 按别名/指纹发文件（自动等待发现匹配）
antify-rs send --text "hi" --host 192.168.1.5:53317   # 直连发送
```

`serve` 常用参数：`--port` / `--alias` / `--dir <下载目录>` / `--no-multicast` / `--ui-port <本机面板端口>`（面板仅绑 127.0.0.1；HTTPS 协议端口对局域网只收发文件，不暴露面板 API）。

桌面窗口即控制台：左侧深色栏自动列出附近设备（微信会话列表式，第二行是最近一条消息预览；
拖文件到设备行即发送），右侧为聊天式会话（参考微信「文件传输助手」）——
文字为彩色气泡、图片直接显示缩略图（点击全屏查看）、文件为中性卡片，头像在消息外侧、时间按间隔居中分组；
收到的文字消息直接显示内容（按官方特征识别：<uuid>.txt 或 FileDto.preview 内嵌正文，
≤64KB 不落盘，带「复制」），其余 .md/.txt 等文本文件一律按文件卡片（文件名 + 图标 + 大小）落盘、可一键「显示」，
活动传输是会话流内吸顶进度卡，最下面是聊天输入条（📎 选文件 / Enter 发文本）；
无设备时两侧同示「等待设备上线」；深色模式跟随系统。
会话记录保存在内存（重启清空，不落盘）。
数据面板由节点本机 `127.0.0.1:53318`（明文 HTTP，仅本机）提供，LocalSend 协议走标准 `53317` HTTPS。

---

## 协议实现要点（对照 LocalSend v2）

| 环节 | 实现 |
| --- | --- |
| 发现 | UDP 多播 `224.0.0.167:53317`，启动 100/500/2000ms 三连公告 + 每 120s 续期；听到陌生公告即回 `POST /register` 并**回敬一次公告**（3s 限流），双向发现 ~1s 完成；**多播按接口收发**（物理网卡 + 虚拟机网桥都入组），UTM/Parallels/VMware 共享网络（NAT）里的虚拟机与宿主机也能互相发现 |
| 身份 | 首次生成自签证书存 `~/.antifybot-rs/`（Windows：`%USERPROFILE%\.antifybot-rs\`），指纹 = 证书 DER 的 SHA‑256（大写 hex），跨重启稳定 |
| 接收 | `POST /prepare-upload`（自动接受，单会话，忙时 409）→ `POST /upload?sessionId&fileId&token`（流式落盘 `.part` 后改名，SHA‑256 校验失败 422 可重试 ≤3 次）→ `POST /cancel` |
| 发送 | 同一套端点反向使用；文件流式上传（ReaderStream），边读边转发不占内存 |
| 健壮性 | 发送方崩溃留下的会话 60s 无活动自动回收；同名文件自动 `名 (n).ext` 改名不覆盖 |

### 跨平台要点

- TLS 全线 ring 后端：无 OpenSSL / CMake / aws-lc 依赖，Windows 构建零额外工具
- Windows 无 `SO_REUSEPORT`，由 `SO_REUSEADDR` 覆盖同机共存语义
- HTTPS 端口被官方 LocalSend 占用时自动顺延（公告携带真实端口）；多播端口被独占时退化为只公告
- 「显示文件」三平台定位：Finder / 资源管理器(`explorer /select,`) / `xdg-open`
- 首次运行防火墙弹窗请点「允许」；未签名二进制 SmartScreen 提示「仍要运行」

---

## 端到端测试

```bash
# 需先启动节点（GUI 或 antify-rs serve）
python3 tests/fake_localsend_test.py
```

内置一个 Python 伪装 LocalSend 设备（自签 HTTPS + 多播公告 + 完整 v2 端点），16 项断言覆盖：
双向多播发现 / register 礼仪、发文本（面板→协议全链路）、收 3MB 文件（含 SHA‑256 校验落盘）、
403 错 token、422 错摘要、409 会话占用、cancel 释放、同名改名。

## 结构

```
rust/
├── src/            antify-rs 节点库 + CLI（lib.rs 为库入口，main.rs 为 CLI）
│   ├── config.rs   身份/证书/指纹持久化
│   ├── discovery.rs UDP 多播发现（多接口收发）+ register 回礼 + 回敬公告
│   ├── server.rs   axum：LocalSend v2 端点（/info /register /prepare-upload /upload /cancel）+ 面板 API
│   ├── client.rs   发送端（prepare-upload → 流式 upload，断点计数）
│   ├── state.rs    共享状态：设备表 / 会话 / 进度 / 事件流
│   └── ui.rs       面板分栏页（深色侧栏设备列表 + 聊天式会话视图，零外部依赖）
├── gui/            Tauri v2 桌面壳（窗口加载本机面板，后台跑节点）
├── dist-windows/   Windows 下载说明（exe 由 CI 产出）
├── build-app.sh    macOS 打包脚本 → AntifyBot.app
├── build-win.ps1   Windows 构建脚本
└── tests/          伪设备端到端测试
.github/workflows/build.yml   CI：双平台构建 → Artifacts / Releases
```

技术栈：Rust · tokio · axum 0.8 · axum-server(rustls/ring) · reqwest · rcgen · clap 4 · Tauri 2。

---

🐜 *蚂蚁不问来路，进窝就是一家人。*
