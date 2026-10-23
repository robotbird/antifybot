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

桌面窗口即控制台：左侧浅灰栏（ChatGPT 式）自动列出附近设备（微信会话列表式，第二行是最近一条消息预览；
拖文件到设备行即定向发送；**离线设备不消失**——置灰排后、显示「最后可见」时间，hover ✕ 可移除），
**整个窗口任意位置都可拖放**——松手即发当前会话（无会话时唯一设备直发、多台弹窗选择），拖动中全窗高亮、顶部提示发送目标，
右侧为聊天式会话（ChatGPT 式灰白）——
右侧为聊天式会话（ChatGPT 式灰白）——
出站消息为灰底圆角气泡、入站为无气泡纯文本（无头像），图片直接显示缩略图（点击全屏查看）、文件为中性卡片，时间按间隔居中分组；
收到的文字消息直接显示内容（按官方特征识别：<uuid>.txt 或 FileDto.preview 内嵌正文，
≤64KB 不落盘，带「复制」），其余 .md/.txt 等文本文件一律按文件卡片（文件名 + 图标 + 大小）落盘、可一键「显示」，
活动传输是会话流内吸顶进度卡，最下面是聊天输入条（📎 选文件 / 选文件夹 / 剪贴板 / Enter 发文本）；
无设备时两侧同示「等待设备上线」。
**离线也能发**：出站消息带送达状态（⏳ 发送中 → ✓ 已送达 / ⚠ 未送达），对方离线时 ~5 秒内标 ⚠，
文本与原生选择器选出的文件可点「重试」（从原路径重发），拖拽/剪贴板中转的文件无源路径、标 ⚠ 不带重试。
**关窗不退出**：点窗口 ✗ 收进系统托盘、节点常驻继续收发；托盘左键唤起/收起面板，菜单含「显示面板 / 退出」（macOS 点 Dock 图标亦可唤起）。
**消息提醒**：收到消息 / 文件、发送失败或后台大件（>10s）送达完成时弹系统通知（macOS 通知中心 / Windows Toast / Linux notify-send）；
面板正被看着时自动静音，收进托盘或切到后台约 6 秒后自动恢复提醒；同一发送方 3 秒内连发（整夹接收）只弹第一条。
**设置面板**（侧栏左下角 ⚙ 进入）：**通用**——主题（跟随系统 / 浅色 / 深色）与语言（中文 / English）；
**存储**——默认保存地址（原生目录选择器或粘贴路径，即时生效并持久化）；
**关于**——版本号、检查更新（GitHub Releases，走系统代理；有新版可在系统浏览器打开下载页）与节点信息（别名 / 端口 / 指纹 / 协议）。
偏好持久化在 `localStorage`（主题 / 语言）与 `~/.antifybot-rs/config.json`（别名 / 保存目录）；
会话与设备持久化在 `~/.antifybot-rs/chat.db`（SQLite，消息保留最近 5000 条，重启全在）。
数据面板由节点本机 `127.0.0.1:53318`（明文 HTTP，仅本机）提供，LocalSend 协议走标准 `53317` HTTPS。

---

## 只读文件夹共享（按目录获取 / 断点续传）

两台电脑均需运行包含此功能的新版 AntifyBot；这是私有扩展，不是 FTP 服务，官方 LocalSend 只能收到共享地址文本，不能浏览目录。

1. 共享方在对方会话中点击 **文件夹按钮**，选择本机文件夹并确认只读共享。不会递归上传文件，而是发送 `antify-share://设备指纹/随机令牌` 地址。
2. 接收方点击消息里的 **浏览共享目录**，逐层打开子目录，点击文件旁的 **下载 / 继续**。每页最多 200 项，支持下一页，目录总文件数不受原来 1000 个文件限制。
3. 文件保存在当前保存目录的 `Shared-<共享令牌>/原相对路径` 下，保留目录结构；已有同名文件不覆盖，会自动另存。
4. 网络异常会自动重试当前块 3 次；仍失败、暂停或程序重启后，重新打开同一地址、点击同一个文件下载即可续传。暂存保存在保存目录的 `.antify-pulls/`；不要删除它或更改保存目录，否则会从头下载。
5. 输入栏 **☷** 打开“我的共享”，可查看地址、撤销共享，或粘贴地址浏览。共享配置保存在 `~/.antifybot-rs/shares/`，重启仍有效；共享方必须保持在线、源目录仍在原位置。

安全与校验：共享是**只读**的，随机地址即访问凭证，持有地址的人可以读取整个共享目录内的常规文件（**包括隐藏文件**），请勿共享敏感目录或转发地址。不遍历子符号链接，不允许 `..` 等越界路径。下载端校验对端 TLS 证书指纹；每个 4 MiB 块及最终整文件均校验 SHA-256。源文件改变后不会沿用旧内容断点；撤销共享后后续目录/分块请求立即失效（已在传输的块可能完成）。暂停在分块边界生效。

旧的主动推送接口、拖放和 LocalSend 兼容传输保持不变，其批量文件限制不影响此共享模式。当前支持逐文件按需下载，不自动镜像整棵目录，也不支持上传、删除远端文件。

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

文件夹共享自动测试（独立临时目录，不影响现有配置）：

```bash
cd rust
cargo test -p antify-rs
```

覆盖超过 1000 文件的分页、中文路径、越界与符号链接拒绝、失效令牌、撤销、源版本变化、真实自签 TLS 下载、中断块截断续传、空文件及同名不覆盖。

## 结构

```
rust/
├── src/            antify-rs 节点库 + CLI（lib.rs 为库入口，main.rs 为 CLI）
│   ├── config.rs   身份/证书/指纹持久化
│   ├── discovery.rs UDP 多播发现（多接口收发）+ register 回礼 + 回敬公告
│   ├── server.rs   axum：LocalSend v2 端点（/info /register /prepare-upload /upload /cancel）+ 面板 API
│   ├── client.rs   发送端（prepare-upload → 流式 upload，断点计数）
│   ├── shares.rs   只读目录映射 / 分页浏览 / 校验分块下载 / 拉取断点续传
│   ├── state.rs    共享状态：设备表 / 会话 / 进度 / 事件流
│   └── ui.rs       面板分栏页（ChatGPT 式浅灰侧栏 + 聊天式会话视图 + 设置面板，主题/双语，零外部依赖）
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
