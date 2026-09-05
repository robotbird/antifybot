# 🐜 AntifyBot 局域网快传

> 同一内网下，电脑与手机之间免注册互传文件与消息 —— 以对话形式呈现。
> Windows / macOS / Linux / iOS / Android，只要浏览器能打开就能用。

**无需注册、无需登录、无需公网。** 设备接入同一个 Wi‑Fi / 网线内网，打开网页即可互发消息与文件。协议与架构参考了 [LocalSend](https://localsend.org/) 的多播发现思路与本仓库姊妹项目 earthchat 的实时对话实现。

---

## 特性

- 💬 **对话式界面** —— 类 IM 的气泡聊天：文字、图片缩略图、文件卡片、打字中指示、消息时间分隔线
- 📎 **文件互传** —— 点击 / 拖拽 / 粘贴发送，8MB 分块断点续传（乱序分块 409 拒绝并纠正），上传带"蚁径"进度与实时速率
- 🖼 **图片灯箱 / 视频拖动** —— 图片内联缩略图 + 点击看大图；下载走 HTTP Range（206），手机端视频可随意拖进度
- 📡 **自动发现同网设备** —— UDP 多播 beacon（LocalSend 风格），无需手输 IP；另提供 CLI 扫描器 `npm run discover`
- 🔗 **扫码加入** —— 页内一键展示 `http://<局域网IP>:3777` 二维码，手机扫码即入
- 🕶 **明暗双主题** —— 跟随系统，移动端安全区适配，可"添加到主屏幕"当 PWA 用
- 🔒 **零依赖前端** —— 原生 ES Module，无构建步骤、无外部 CDN；所有消息经 `textContent` 渲染，杜绝 XSS
- 🏷 **匿名身份** —— localStorage 随机身份（可改名），服务端按设备指纹生成专属色相，每台设备的气泡/蚂蚁头像颜色稳定不撞

## 快速开始

```bash
npm install
npm start          # 默认 0.0.0.0:3777，可用 PORT=8080 覆盖
```

任意一台内网设备浏览器打开：

```
http://<运行 npm start 那台机器的局域网IP>:3777
```

页内点右上角二维码按钮，手机扫码加入即可互传。查看本机 IP：macOS/Linux `ifconfig | grep inet`，Windows `ipconfig`。

### CLI 设备扫描器

```bash
npm run discover
```

加入 UDP 多播组，实时列出当前内网中所有 AntifyBot 节点（名称 / IP / 端口），可用于排障或无浏览器环境探测。

## 协议

### 发现：UDP 多播 beacon

| 项 | 值 |
| --- | --- |
| 组播组 | `239.71.66.1:45666` |
| 广播间隔 | 每 `3s` 一次 |
| 邻居 TTL | `10s` 未再发声视为下线 |
| TTL(hop) | 1（不跨路由器，纯内网） |

beacon 为一行 JSON：

```json
{"v":1,"app":"antifybot","id":"b7f0…","name":"xiepengdeMacBook-Air","port":3777,"ts":1725400000000}
```

同 `app` 且版本兼容的包才会被收录；节点 id 写在 `~/.antifybot-node-id`。

### 上传：三段式分块

```
POST /api/upload/init            { name, size, mime }
     → { fileId, chunkSize: 8MB, maxFileBytes }
PUT  /api/upload/chunk/:fileId   body=原始字节流, 头 x-offset=N
     → 服务端已收字节数（客户端据此纠偏；offset 不匹配返回 409）
POST /api/upload/complete/:fileId
     → 文件落盘 + sidecar 元数据 + Socket.IO 广播文件消息
```

客户端每块最多重试 3 次；断线重连后可从服务端已收偏移续传。

### 下载

`GET /files/:fileId/<原名>`：非白名单 MIME 一律 `attachment` 强制下载，文件名按 RFC 5987 以 UTF‑8 编码（中文名不乱码）；支持 `Range` 请求返回 `206`。

### 实时消息：Socket.IO

`hello / hello-ack / message / typing`；后加入的设备自动收到最近的历史消息与当前在线名单。

## 限制（默认值）

| 项 | 值 | 覆盖方式 |
| --- | --- | --- |
| 单文件上限 | 2 GB | `MAX_FILE_MB=512 npm start` |
| 单条文字 | 4000 字符 | — |
| 内存历史 | 300 条（文件元数据落盘不受限，重启可恢复下载） | — |
| 消息限流 | 每连接 10s 内 25 条 | — |
| 端口 | 3777 | `PORT=8080 npm start` |

## 安全说明

- **纯内网**：多播 TTL=1，服务监听仅面向局域网场景，请勿暴露到公网
- **免注册的代价**：同网段任何设备都能加入房间并收发消息，敏感内网请自行加反向代理鉴权
- **XSS 防护**：前端所有用户内容经 `textContent` / `createElement` 渲染；服务端仅对图片等白名单 MIME 放行 inline，其余强制下载；文件名经 sanitize
- 无 Cookie、无追踪，身份仅存于本浏览器 localStorage

## 架构

```
server.js            Express + Socket.IO：静态托管 / 聊天信令 / 上传下载 / 限流 / QR
lib/discovery.js     UDP 多播发现（可独立 require）
bin/antify-discover.js  CLI 扫描器
public/              前端（原生 ES Module，无构建）
  js/main.js         会话编排：socket、发送、拖拽/粘贴、灯箱、改名、QR
  js/chat.js         气泡 / 文件卡片 / 蚁径上传进度渲染
  js/api.js          分块上传 HTTP 客户端（3× 重试）
  js/identity.js     localStorage 匿名身份 + 平台识别 + 设备色相
  js/util.js         DOM 构建 h()、格式化、蚂蚁 SVG、toast
files/               收到的文件 + .json sidecar（重启后仍可下载）
antify-p2p.html      零服务端 P2P 单文件版（WebRTC DataChannel，详见上文专节）
test/smoke.js        13 项集成冒烟（含 9.3MB 分块上传、Range、409、UDP 发现、重启恢复）
```

技术栈：Node.js ≥18 · Express 4 · Socket.IO 4 · 原生 Web 前端（零构建、零外部资源）。

## 测试

```bash
npm test    # 13 项集成冒烟：HTTP / Socket.IO / 分块上传 / Range / UDP 发现 / 重启恢复
```

---

## 🐜 零服务端 P2P 单文件版（`antify-p2p.html`）

不想跑 `npm start`？仓库根目录的 **`antify-p2p.html`** 是一个零依赖单文件页：两台设备的浏览器之间
WebRTC DataChannel 直连，**没有任何服务器程序参与**，消息与文件不落第三方。

适合：临时把一台 Mac 和一台 Windows 互连、U 盘/微信只带得动一个文件的场景。

### 使用步骤（Mac ↔ Windows）

1. **把这一个 HTML 文件带到两台设备上**（各一份拷贝）——U 盘、微信文件传输、AirDrop 或先起服务器版下载都行；
2. 双击打开（Chrome / Edge / Safari 均可，`file://` 直接可用，无需联网）；
3. 各自填名字后按页面三步握手：
   - **STEP 1** A 机点「我是发起方，生成配对码」→ 复制那串 `AB1…` 文本（微信/邮件/手动誊抄均可带给 B）；
   - **STEP 2** B 机粘贴配对码 → 点「生成回执码」→ 复制回执码带回 A；
   - **STEP 3** A 机粘贴回执码 → 完成配对，进入对话界面；
4. 之后就像聊天软件：发消息、点 📎 或拖拽文件进窗口即可互传；收到的文件点卡片上的 ⬇ 保存
   （Chrome/Edge 弹系统保存框，其余走浏览器下载）。

### 与服务器版的差异

| | 服务器版（`npm start`） | P2P 单文件版 |
| --- | --- | --- |
| 需要运行程序 | 一台机器跑 Node | **完全不需要** |
| 设备发现 | UDP 多播自动发现 + 扫码 | 复制粘贴配对码（一次性） |
| 同时在线 | 多台设备群聊 | **仅两台** 点对点 |
| 断点续传 | ✅ 8MB 分块可续传 | ✗ 断线需重新配对 |
| 文件去向 | 服务端落盘，可反复下载 | 接收方内存暂存，点保存落盘 |
| 单文件建议 | ≤ 2GB | ≤ 1GB（接收在内存中） |

### macOS 应用（可选，`mac/`）

把单文件页包成原生 `AntifyBot.app`（WKWebView 壳，约 680KB，无需 Xcode，系统自带 swiftc 编译）：

```bash
bash mac/build.sh     # 产出 mac/AntifyBot.app
open  mac/AntifyBot.app
```

- 双击即用：文件选择 / 收到的文件保存走系统原生面板（NSOpenPanel / NSSavePanel）；
- 页面本体仍随包内嵌，升级时重跑 build 即可（会重新打包 `antify-p2p.html`）；
- 自检：`ANTIFY_SELFCHECK=1` 校验 delegate 选择器、`ANTIFY_AUTOTEST=1` 自动生成配对码验证 WebRTC 链路。

### 注意事项

- 两台设备须在**同一局域网**（同一 Wi‑Fi / 网线）；跨网段、开了 AP 隔离或企业防火墙拦 UDP 时会握手失败；
- 配对码包含本机网络地址，仅在两台设备间交换即可，请勿发给无关第三方；
- 关闭页面即断开，对方会看到断线横幅，点「重新配对」重来一次即可。

---

## 🐜🐜 Rust 版：LocalSend v2 协议节点 + Tauri 桌面应用（`rust/`）

Rust 重写版，**与 [LocalSend](https://localsend.org) 官方 App 直接互通**：手机上装 LocalSend，
电脑上跑 AntifyBot-Rust，互相就能看见、互发文件。协议实现遵循
[localsend/protocol](https://github.com/localsend/protocol) v2（多播发现 + HTTPS 收发 + 自签证书指纹）。

### 两种用法

**1. Tauri 桌面应用（推荐）**

```bash
cd rust
cargo build --workspace --release
bash build-app.sh          # 产出 rust/AntifyBot.app（debug 可直接 cargo run -p antify-gui）
open AntifyBot.app
```

窗口即控制台：设备卡片、发文本/发文件、接收进度、事件日志一屏全览。
数据面板由节点本机 `127.0.0.1:53318`（明文 HTTP，仅本机）提供，LocalSend 协议走标准 `53317` HTTPS。

**2. 无头 CLI 节点**

```bash
cargo build --release
./target/release/antify-rs serve                      # 常驻节点 + 面板 https://127.0.0.1:53317
./target/release/antify-rs discover                   # 扫描当前网段的 LocalSend 设备
./target/release/antify-rs send photo.jpg --to 手机    # 按别名/指纹发文件（自动等待发现）
./target/release/antify-rs send --text "hi" --host 192.168.1.5:53317   # 直连发送
```

`serve` 常用参数：`--port` / `--alias` / `--dir <下载目录>` / `--no-multicast` / `--ui-port <本机面板>`。

### Windows

**开箱即用**：`rust/dist-windows/` 里有现成的 x86-64 可执行文件（从 macOS 交叉编译产出）——

- `antify-gui.exe`：双击打开桌面窗口（Tauri + WebView2，Win11 自带运行时）
- `antify-rs.exe`：命令行节点，`.\antify-rs.exe serve` / `discover` / `send`

自行构建（装好 Rust MSVC 工具链后）：

```powershell
cd rust
powershell -ExecutionPolicy Bypass -File build-win.ps1
```

跨平台要点：TLS 全线用 ring 后端（无 OpenSSL / CMake 依赖）；Windows 无 `SO_REUSEPORT`，由 `SO_REUSEADDR` 覆盖同机共存语义；HTTPS 端口被官方 LocalSend 占用时自动顺延（公告携带真实端口）；「显示文件」用资源管理器定位。首次运行防火墙弹窗请点「允许」。

### 协议实现要点（对照 LocalSend v2）

| 环节 | 实现 |
| --- | --- |
| 发现 | UDP 多播 `224.0.0.167:53317`，启动 100/500/2000ms 三连公告 + 每 120s 续期；听到陌生公告即回 `POST /register` 并**回敬一次公告**（3s 限流），双向发现 ~1s 完成 |
| 身份 | 首次生成自签证书存 `~/.antifybot-rs/`，指纹 = 证书 DER 的 SHA‑256（大写 hex），跨重启稳定 |
| 接收 | `POST /prepare-upload`（自动接受，单会话，忙时 409）→ `POST /upload?sessionId&fileId&token`（流式落盘 `.part` 后改名，SHA‑256 校验失败 422 可重试 ≤3 次）→ `POST /cancel` |
| 发送 | 同一套端点反向使用；文件流式上传（ReaderStream），边读边转发不占内存 |
| 健壮性 | 发送方崩溃留下的会话 60s 无活动自动回收；同名文件自动 `名 (n).ext` 改名不覆盖 |

### 端到端测试

```bash
# 需先启动节点（GUI 或 antify-rs serve）
python3 tests/fake_localsend_test.py
```

内置一个 Python 伪装 LocalSend 设备（自签 HTTPS + 多播公告 + 完整 v2 端点），16 项断言覆盖：
双向多播发现 / register 礼仪、发文本（面板→协议全链路）、收 3MB 文件（含 SHA‑256 校验落盘）、
403 错 token、422 错摘要、409 会话占用、cancel 释放、同名改名。

### 结构

```
rust/
├── src/            antify-rs 节点库 + CLI（lib.rs 为库入口，main.rs 为 CLI）
│   ├── config.rs   身份/证书/指纹持久化（~/.antifybot-rs/）
│   ├── discovery.rs UDP 多播发现 + register 回礼 + 回敬公告
│   ├── server.rs   axum：LocalSend v2 端点（/info /register /prepare-upload /upload /cancel）+ 面板 API
│   ├── client.rs   发送端（prepare-upload → 流式 upload，断点计数）
│   ├── state.rs    共享状态：设备表 / 会话 / 进度 / 事件流
│   └── ui.rs       面板单页（暖纸蚁巢主题，零外部依赖）
├── gui/            Tauri v2 桌面壳（窗口加载本机面板，后台跑节点）
├── dist-windows/   Windows x86-64 可执行文件（antify-gui.exe / antify-rs.exe）
├── build-app.sh    macOS 打包脚本 → AntifyBot.app
├── build-win.ps1   Windows 构建脚本
└── tests/          伪设备端到端测试
```

技术栈：tokio · axum 0.8 · axum-server(rustls) · reqwest · rcgen · clap 4 · Tauri 2。

---

🐜 *蚂蚁不问来路，进窝就是一家人。*
