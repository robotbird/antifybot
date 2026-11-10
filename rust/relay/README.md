# Antify Relay

AntifyBot 的极简 Rust 中转服务：单进程、SQLite、磁盘附件。服务器只部署静态 Linux 可执行程序、配置和 systemd 单元，不需要 Rust、Docker 或外部数据库。

桌面客户端从 v1.5.0-rc.1 起支持在「设置 → 远程连接」保存服务器地址和设备令牌，可通过现有会话界面收发文字、文件。

## 功能与范围

- 每台设备独立 Bearer 令牌；服务端配置只保存 SHA-256 摘要。配置中的设备属于同一个可信小组，可互相发送消息。
- HTTP / WebSocket 发送消息，服务端事务落库后返回 `accepted`。
- 接收设备保存消息后确认，发送者收到 `delivered`；发送者也需确认该回执。
- 未确认消息和回执会重复推送，客户端必须按消息 UUID 去重。每次最多返回 128 条消息和 128 条回执，确认后继续取。
- 断开重连后继续补收。默认消息保留 **7 天**，每个发送设备最多 10000 条；超过保留期将清理，包括未送达消息。
- 文件流式上传下载，SHA-256 校验，可查元数据。仅上传者与指定接收者可读取附件。
- 默认单文件 **100 MiB**、附件总配额 **1 GiB**、最多 10000 个附件，保留 **24 小时**；上传最长 300 秒。上传时必须提供 `Content-Length`，可选 `X-File-SHA256`。
- 心跳与在线状态（以 WebSocket 连接为准），每设备最多两条连接。

`delivered` 表示接收端已确认保存消息，不代表用户已读，也不代表附件已经下载。暂无群聊、端到端加密、文件断点续传、自动客户端发现或管理后台。

## 本机开发

在仓库根目录生成配置，工具不会覆盖已有文件：

```bash
python3 rust/relay/deploy/generate_config.py \
  --output .local/relay \
  --data-dir ./relay-data
cd rust
ANTIFY_RELAY_CONFIG=../.local/relay/server.json cargo run -p antify-relay
```

`server.json` 保存服务端配置；`credentials.json` 保存两台设备的明文令牌，仅供客户端使用。这两个文件位于忽略目录且权限为 0600。不要提交或公开客户端凭证。

服务端默认监听 `127.0.0.1:53319`。修改设备配置、撤销令牌需重启服务。公网访问应通过 HTTPS 反向代理；没有域名时可使用 SSH 隧道。

## API

除 `/health` 外，所有接口均需要 `Authorization: Bearer <token>`。

| 方法 | 地址 | 用途 |
| --- | --- | --- |
| GET | `/health` | 健康状态与版本 |
| GET | `/v1/devices` | 已配置设备与在线状态，不返回令牌摘要 |
| GET | `/v1/ws` | WebSocket 连接 |
| POST | `/v1/messages` | 保存消息，返回 accepted |
| GET | `/v1/messages` | 获取自己的待收消息及待确认送达回执 |
| POST | `/v1/messages/{id}/ack` | 接收者确认消息；发送者确认 delivered 回执 |
| POST | `/v1/files?to=desktop-b&name=test.bin` | 原始二进制上传，不使用 multipart |
| GET | `/v1/files/{id}/info` | 文件名、大小、SHA-256、过期时间 |
| GET | `/v1/files/{id}` | 下载附件 |

HTTP 发送格式：

```json
{
  "id": "7eb21880-2b85-4c14-8ad4-e9d62c9c8ca4",
  "to": "desktop-b",
  "body": {"kind": "text", "text": "你好"}
}
```

`id` 由客户端生成，重试必须沿用同一 UUID 和相同内容；发送者由令牌决定。文字正文限 32 KiB。HTTP JSON 请求 / WebSocket 消息限 64 KiB。

发送文件时先上传，得到文件 ID，再发送 `{"kind":"file","file_id":"返回的文件 UUID"}` 作为 `body`。

WebSocket 客户端事件：

```json
{"type":"send","message":{"id":"7eb21880-2b85-4c14-8ad4-e9d62c9c8ca4","to":"desktop-b","body":{"kind":"text","text":"你好"}}}
{"type":"ack","id":"7eb21880-2b85-4c14-8ad4-e9d62c9c8ca4"}
```

服务端返回 `ready`、`accepted`、`message`、`delivered` 或 `error`。完整类型见 `src/protocol.rs`。收到消息后先写入客户端本地数据库，再确认。`accepted` 的恢复依赖发送者重试同一消息；`message` 和 `delivered` 在保留期内持续补发直到确认。网络传输和磁盘内容没有端到端加密。

## 本机编译 Linux 静态程序

macOS Apple Silicon 可通过 Zig 交叉编译，无需在服务器安装构建工具：

```bash
rustup target add x86_64-unknown-linux-musl
python3 -m pip install --target .local/relay/build-tools ziglang==0.14.1
chmod +x rust/relay/deploy/zig-cc.sh rust/relay/deploy/zig-ar.sh
ANTIFY_ZIG="$PWD/.local/relay/build-tools/ziglang/zig" \
  bash rust/relay/deploy/build-linux.sh
```

首次编译前执行 `cd rust && cargo fetch --locked` 下载依赖。构建脚本随后使用离线模式，产物在 `rust/target/x86_64-unknown-linux-musl/release/antify-relay`。

上传该程序、`server.json` 和 `deploy/` 中的 `install.sh`、`antify-relay.service` 到服务器临时目录。服务端配置的 `data_dir` 应为 `/var/lib/antify-relay`；可省略 `--data-dir` 重新生成一套独立部署配置。

以 root 执行：

```bash
bash install.sh ./antify-relay ./server.json ./antify-relay.service
```

安装脚本不会覆盖已有配置。服务以专用非 root 用户运行，使用两个异步工作线程、最多四个阻塞线程，systemd 内存上限 128 MiB，开机启动、异常重启。

| 路径 | 内容 |
| --- | --- |
| `/opt/antify-relay/bin/antify-relay` | 静态执行程序 |
| `/etc/antify-relay/server.json` | 配置与令牌摘要 |
| `/var/lib/antify-relay/relay.db` | SQLite 消息和文件元数据 |
| `/var/lib/antify-relay/files/` | 附件 |
| `/etc/systemd/system/antify-relay.service` | 启动配置 |

后续更新会保存旧二进制为 `antify-relay.previous`。运行状态通过 `systemctl status antify-relay` 查看，日志通过 `journalctl -u antify-relay` 查看。备份时需包含 SQLite 的一致性备份和附件目录；直接复制正在运行的数据库文件不构成完整备份。

## 验证

```bash
cd rust
cargo test -p antify-relay
cargo clippy -p antify-relay --all-targets -- -D warnings
```

没有域名时，在开发机建立隧道（保持终端运行）：

```bash
ssh -N -L 127.0.0.1:53320:127.0.0.1:53319 用户名@服务器IP
```

另一个终端执行远端测试，仅使用专门准备的测试设备凭证：

```bash
cd rust
ANTIFY_RELAY_TEST_URL=http://127.0.0.1:53320 \
ANTIFY_RELAY_TEST_CREDENTIALS="$PWD/../.local/relay/credentials.json" \
  cargo test -p antify-relay --test e2e deployed_smoke -- --ignored --nocapture
```

测试会在服务端留下少量已确认消息和一个 1 MiB 附件，后者随过期任务清理。客户端完整联调可执行 `cargo test -p antify-rs --test relay_client_test deployed_client_roundtrip -- --ignored`，同样通过以上环境变量指定服务器和凭证文件。
