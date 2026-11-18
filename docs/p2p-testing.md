# AntifyBot 点对点直传联调

客户端版本为 v1.5.0-rc.7，信令服务版本为 v0.3.0。同一局域网优先直传，跨网络默认使用 HTTPS 中转。在「远程连接」启用「跨网络使用点对点直传」可手动选择 WebRTC。

中转使用经设备令牌认证的 WebSocket。文件在客户端与服务器之间通过 TLS 加密，服务器转发数据帧，附件落盘、断点进度和整文件 SHA-256 校验由客户端处理。跨网络中转需要发送端和接收端均在线，离线时任务留在发送端。

## 在 123.56.79.62 更新信令服务

123.56.79.62 已更新至 v0.3.0。健康检查成功，现有令牌通过公网 HTTPS 完成了 WebSocket 握手。升级只替换了 Linux x86_64 静态程序，继续使用原配置、设备令牌和数据目录。再次更新时可执行以下命令。

```sh
scp antify-relay root@123.56.79.62:/tmp/antify-relay-p2p
ssh root@123.56.79.62
cp -p /opt/antify-relay/bin/antify-relay /opt/antify-relay/bin/antify-relay.previous
install -m 0755 /tmp/antify-relay-p2p /opt/antify-relay/bin/antify-relay.new
mv /opt/antify-relay/bin/antify-relay.new /opt/antify-relay/bin/antify-relay
systemctl restart antify-relay
systemctl is-active antify-relay
curl --fail http://127.0.0.1:53319/health
```

若需恢复此前版本，将备份程序恢复到原路径，再重启 `antify-relay`。服务端日常日志可用 `journalctl -u antify-relay -n 100` 检查。

## 配置两台 Mac

1. 两台 Mac 均解压并启动 Apple Silicon 测试包。系统若拦截本地签名测试包，可在确认来源后使用 macOS 的「仍要打开」。
2. 在「远程连接」中使用默认地址 `https://p2p.bjjxysbz.com`。原先使用 `http://127.0.0.1:53320` 的配置会自动切换，继续使用原令牌。首次配置时，在两台 Mac 分别填写各自的令牌并启用连接。
3. 两端上线后检查设备列表。异地发送文件时进度卡显示「HTTPS 中转」。需要测试手动点对点时，开启设置中的直传选项，两端所在网络需允许 UDP 直连。

## M1 和 M5 验收记录

| 编号 | 真机操作与通过条件 |
| --- | --- |
| A1 | 同一 Wi-Fi 双向收发文字及文件，检查「局域网直传」。随后用旧版客户端重复局域网收发。 |
| A2 | 分别连接 Wi-Fi 与手机热点，双向发送文字、选择文件和拖拽文件。检查默认显示「HTTPS 中转」、消息去重和附件未在服务器落盘。 |
| A3 | 发送 2 GiB 文件，两端用 `shasum -a 256 文件名` 比对摘要，用活动监视器记录两端内存峰值。 |
| A4 | 分别断网和重启客户端，观察续传起点与进度，确认完成后的摘要一致。 |
| A5 | 在阻断设备间 UDP、HTTPS 可通的条件下发送，确认默认中转仍可送达。切换手动点对点后，确认超时状态和重试入口可用。 |
| A6 | 接收端退出，发送端发起任务后退出再启动。接收端上线后确认自动发送且只入库一次。 |
| A7 | 同一设备同时出现在局域网和信令列表，核对名称、图标和原会话历史。另用同名的不同身份确认各显示一条记录。 |
| A8 | 暂停传输后修改源文件再重试，确认提示重新选择且未显示送达。测试破坏接收临时文件后，接收端应拒绝完成。 |

实际公网与物理双 Mac 验收待执行。运行环境、选用路径、续传偏移、摘要、内存峰值和结果按各编号记录。

## 本地复测

- Rust 工作区通过 18 项测试。两项需要外部服务的测试默认跳过。
- 双客户端集成覆盖文字、文件选择、拖拽、离线重启、回执去重、旧会话合并、同名设备区分和源文件变更拒绝。测试服务的附件目录和消息接口均未出现 P2P 内容。
- v1.5.0-rc.7 的中转集成测试覆盖 32 MiB 文件在双端重启后的续传和内容一致性，手动点对点另行通过联调。
- 早期点对点版本的 Release 集成测试传输完整 2 GiB 文件。在接收端确认 1,048,576 字节后重启两个客户端实例，发送端从该偏移继续，最终长度和 SHA-256 一致。两次复测中恢复后的传输耗时约 75–79 秒。其中一次用 `/usr/bin/time -l` 测得整个测试进程（两个客户端实例与信令服务）的峰值常驻内存约 25.2 MiB。
- v1.5.0-rc.3 的 32 MiB 联调覆盖接收中的文件信息、文字并行发送和两端重启续传。浏览器验证了接收进度、校验状态和文件名后的实时速度。
- 本机通过现有 LocalSend 直连路径收到了文字和文件。浏览器验证设备列表和会话标题均显示 12 px 小地球，包含悬停提示与辅助功能标签。

集成测试在同一台机器上运行两个独立客户端实例及真实信令服务，禁用 STUN 以获得可重复的测试条件。大文件测试建议使用 release 构建。

```sh
cargo test --manifest-path rust/Cargo.toml --workspace
ANTIFY_P2P_CONCURRENT_TEST=1 cargo test --manifest-path rust/Cargo.toml -p antify-rs --test relay_client_test panel_to_p2p_to_other_client_and_restart -- --nocapture
ANTIFY_P2P_LARGE_TEST=1 cargo test --release --manifest-path rust/Cargo.toml -p antify-rs --test relay_client_test panel_to_p2p_to_other_client_and_restart -- --nocapture
```


## 排查异地传输速度

v1.5.0-rc.4 在发送数据的同时读取写盘回执，仍保留最多 4 MiB 的未确认数据。客户端记录所选的直连候选类型、候选网卡、系统选路网卡，以及首块和慢块的确认耗时。接收端另外记录收包及保存续传位置的耗时。系统选路网卡来自对实际对端候选地址的系统选路探测。

两台 Mac 完成一次复测后，可在各自终端运行以下命令提取本次点对点事件。输出不包含令牌、文件内容和 ICE 地址。

```sh
curl -fsS http://127.0.0.1:53318/api/ui/state | python3 -c 'import json,sys; print("\n".join(e["text"] for e in json.load(sys.stdin)["events"] if e["text"].startswith("点对点")))'
```

同一台 M1 上的两个独立客户端实例完成了 9 MiB 对照传输。两次测试耗时约 6–13 秒，首块写盘及保存续传位置约 0.05–0.09 秒。该结果用于核对客户端和磁盘路径，异地连接仍需用两台 Mac 复测。

```sh
ANTIFY_P2P_DIAG_TEST=1 cargo test --manifest-path rust/Cargo.toml -p antify-rs --test relay_client_test panel_to_p2p_to_other_client_and_restart -- --nocapture
```
