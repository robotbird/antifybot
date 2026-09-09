//! 共享状态：发现的设备、进行中的接收会话、已收文件、事件流
use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;

/// 发现（或手动添加）的远端 LocalSend 设备
#[derive(Clone, Serialize)]
pub struct Device {
    pub fingerprint: String,
    pub alias: String,
    pub ip: IpAddr,
    pub port: u16,
    #[serde(rename = "https")]
    pub https: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_type: Option<String>,
    pub version: String,
    pub download: bool,
    /// 最后一次听到它（多播 / register / 手动添加）的毫秒时间戳
    pub last_seen: u64,
}

impl Device {
    pub fn base_url(&self) -> String {
        format!(
            "{}://{}:{}",
            if self.https { "https" } else { "http" },
            match self.ip {
                IpAddr::V4(v4) => v4.to_string(),
                IpAddr::V6(v6) => format!("[{v6}]"),
            },
            self.port
        )
    }
}

/// 一条待接收文件（v2 FileDto 落到本地后的一半）
#[derive(Clone)]
pub struct IncomingFile {
    pub token: String,
    pub file_name: String,
    pub size: u64,
    pub sha256: Option<String>,
    /// prepare-upload 声明的 MIME
    pub mime: String,
    /// v2 FileDto.preview：官方客户端发文字消息时内嵌正文（区别于普通 .txt 文件）
    pub preview: String,
    pub done: bool,
    /// 整文件单发路径的重试计数（分块路径不递增——每块一个请求会把 3 次上限打爆）
    pub attempts: u8,
    /// 分块路径：暂存区里已验证的字节数（进度展示 + resume-info 数据源）
    pub received: u64,
    /// 分块路径：连续块 hash 不符计数（≥3 视为链路/源损坏，置 done 防死循环）
    pub mismatch_streak: u8,
}

/// 活动接收会话（协议约束：同时仅一个）
pub struct Session {
    pub id: String,
    pub sender_ip: IpAddr,
    pub sender_alias: String,
    /// 发送方 prepare-upload info 里的指纹（会话流归因用；同机测试/NAT 下比 IP 可靠）
    pub sender_fp: String,
    pub files: HashMap<String, IncomingFile>,
    /// 最近一次活动（prepare / upload）；超时未动自动回收，防止发送方崩溃后锁死
    pub last_active: tokio::time::Instant,
}

#[derive(Clone, Serialize)]
pub struct ReceivedFile {
    pub name: String,
    pub size: u64,
    pub at: u64,
    /// 相对下载目录的文件名（用于"在 Finder 中显示"）
    pub file: String,
}

/// 面板/CLI 可见的发送进度（UI 中转 / CLI send 共用）
#[derive(Clone, Serialize, Default)]
pub struct SendProgress {
    pub active: bool,
    pub target_alias: String,
    pub file_name: String,
    pub sent: u64,
    pub total: u64,
    pub started_at: u64,
}

/// 出站图片回显条目（会话流 <img> 取用；与聊天记录同生命周期，仅内存）
pub struct CachedImg {
    pub mime: String,
    pub bytes: Vec<u8>,
}

/// 出站图片缓存总量上限（超出按最旧淘汰）
const IMG_CACHE_TOTAL: usize = 48 * 1024 * 1024;

/// 会话流里的一条消息（右侧聊天视图的数据源；SQLite 持久化，重启不丢）
#[derive(Clone, Serialize, Default)]
pub struct ChatMsg {
    pub id: u64,
    /// true = 我方发出
    pub out: bool,
    /// 对端指纹（收到时按来源 IP 反查设备表；查不到退化为 IP 字符串）
    pub peer: String,
    /// 发生时的对端别名快照（设备改名/离线后气泡仍可读）
    pub peer_alias: String,
    /// "text" | "file"
    pub kind: String,
    pub text: String,
    pub name: String,
    pub size: u64,
    pub at: u64,
    /// 已收文件名（相对下载目录，可「显示」）；出站为空
    pub file: String,
    /// 出站专用："sending" | "ok" | "fail"；入站恒空（协议无已读回执）
    pub status: String,
    /// 出站文件的源绝对路径（重试时从原路径重读重发）；流式中转/文本为空
    pub src_path: String,
}

/// 原生文件选择器的接入方式（macOS 的 rfd 同步 API 依赖 NSApplication）：
/// - Inline：GUI（Tauri 已运行 NSApp）—— 工作线程直接调，rfd 自动派发主线程
/// - Bridge：CLI `serve`（无 NSApp，非主线程调用会 panic）—— 请求经 mpsc 转主线程代调
pub enum Picker {
    Inline,
    Bridge(std::sync::mpsc::Sender<PickJob>),
}

/// 主线程代调任务：folder=false 选文件（可多选），true 选文件夹
pub struct PickJob {
    pub folder: bool,
    pub reply: tokio::sync::oneshot::Sender<Option<Vec<PathBuf>>>,
}

pub struct AppState {
    pub identity: crate::config::Identity,
    /// 当前保存目录（初值 = identity.download_dir；设置面板可改，写穿 config.json）
    pub download_dir: tokio::sync::RwLock<PathBuf>,
    /// SQLite（消息真源 + 设备写穿副本），见 db.rs
    pub db: crate::db::Db,
    /// 构造时 Inline；CLI serve 在 start_node 返回后换成 Bridge（tokio Mutex 便于原地换）
    pub picker: Mutex<Picker>,
    pub devices: Mutex<HashMap<String, Device>>,
    pub session: Mutex<Option<Session>>,
    pub received: Mutex<Vec<ReceivedFile>>,
    pub events: Mutex<VecDeque<(u64, String)>>,
    pub sending: Mutex<SendProgress>,
    /// 出站图片回显缓存（chat 消息 id → 图）：出站文件不落盘，图片留内存供面板预览；
    /// 接收图片在下载目录里，直接读盘不进缓存。id 即 DB 消息主键，跨重启稳定
    pub img_cache: Mutex<std::collections::BTreeMap<u64, CachedImg>>,
    /// 面板中转发送的实时字节计数（SendProgress 里的是快照，此处原子累加）
    pub relay_bytes: AtomicU64,
    /// 接收侧当前文件进度（upload 处理器实时更新）
    pub rx_bytes: AtomicU64,
    pub rx_total: AtomicU64,
    pub rx_name: Mutex<String>,
    /// 分块写入段互斥：发送端超时重试的两请求并发到达时串行化，
    /// 配合「offset 失衡 → 截断重写」幂等规则收敛
    pub rx_write: Mutex<()>,
    /// 出站传输取消标志（面板「取消」按钮置位；client::send 的块/轮循环检查）
    pub send_cancel: std::sync::atomic::AtomicBool,
    /// 面板焦点快照：(是否可见且聚焦, 上次上报时刻)。面板随 /api/ui/state 轮询捎带
    /// 上报——持续在报且聚焦 = 用户正看着会话流，节点静音系统通知；
    /// 收进托盘 / 切后台后 WebView 挂起自然停报，超窗自动回落到「提醒」。
    /// None（CLI serve 无面板打开）恒视为不在看
    pub panel_focus: std::sync::Mutex<Option<(bool, std::time::Instant)>>,
    /// 接收侧取消后的拒收窗口：发送端 IP → 拒收截止时刻。
    /// 窗口内 prepare-upload 一律 204（官方协议的「拒收」语义，
    /// 我方发送端据此终止不重试），否则自动接受会让取消形同虚设
    pub declined: Mutex<HashMap<IpAddr, std::time::Instant>>,
}

pub type Shared = Arc<AppState>;

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

impl AppState {
    /// 设备入表 + 写穿 DB（发现 / register / 手动添加统一走这里；
    /// 新数据整行覆盖旧值——IP/别名变化自然刷新；DB 失败仅记事件不拦内存）
    pub async fn upsert_device(&self, d: Device) {
        if let Err(e) = self.db.upsert_device(&d) {
            self.log_event(format!("设备「{}」入库失败: {e:#}", d.alias)).await;
        }
        self.devices
            .lock()
            .await
            .insert(d.fingerprint.clone(), d);
    }

    /// 移除设备（内存 + DB）；消息表保留——对方再上线时历史会话还在
    pub async fn remove_device(&self, fp: &str) {
        let alias = self.devices.lock().await.remove(fp).map(|d| d.alias);
        if let Err(e) = self.db.remove_device(fp) {
            self.log_event(format!("移除设备入库失败: {e:#}")).await;
        }
        if let Some(a) = alias {
            self.log_event(format!("已移除设备「{a}」（重新上线会自动出现）")).await;
        }
    }

    /// 常驻节点启动恢复：设备表回填内存 → 上次在途的出站消息落 fail → 历史裁剪。
    /// 一次性命令（discover/send）不调用——避免把常驻节点在途的 sending 误标 fail。
    pub async fn load_persisted(&self) {
        let devs = self.db.load_devices();
        if !devs.is_empty() {
            let mut map = self.devices.lock().await;
            for d in devs {
                map.insert(d.fingerprint.clone(), d);
            }
        }
        let n = self.db.fail_pending_sending();
        if n > 0 {
            self.log_event(format!("{n} 条上次未完成的消息已标记未送达（可点击重试）")).await;
        }
        self.db.prune();
    }

    /// 拒收窗口是否命中（顺手清掉已过期的条目）
    pub async fn is_declined(&self, ip: &IpAddr) -> bool {
        let mut d = self.declined.lock().await;
        let now = std::time::Instant::now();
        d.retain(|_, until| *until > now);
        d.contains_key(ip)
    }

    pub async fn log_event(&self, text: impl Into<String>) {
        let line = text.into();
        println!("[{}] {}", humantime(now_ms()), line);
        let mut ev = self.events.lock().await;
        ev.push_back((now_ms(), line));
        while ev.len() > 200 {
            ev.pop_front();
        }
    }

    /// 会话流追加一条（SQLite 持久化，重启不丢）；返回消息 id（出站图片回显缓存以此为主键）。
    /// DB 写入失败仅记事件并返回 0（set_msg_status(0) 自然 no-op，消息不进面板但流程不断）
    pub async fn push_chat(&self, m: ChatMsg) -> u64 {
        match self.db.insert_msg(&m) {
            Ok(id) => id,
            Err(e) => {
                self.log_event(format!("消息入库失败: {e:#}")).await;
                0
            }
        }
    }

    /// 面板是否正被看着（可见且聚焦，且 6 秒内仍在轮询上报）。
    /// 通知静音判定：持续在报才静音——WebView 挂起 / 面板压根没开都算「不在看」
    pub fn panel_watching(&self) -> bool {
        self.panel_focus
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .map(|(f, at)| f && at.elapsed() < std::time::Duration::from_secs(6))
            .unwrap_or(false)
    }

    /// 出站消息状态流转：sending → ok / fail。
    /// 翻转即提醒时机：失败总是弹系统通知（面板不在看时）；成功只对
    /// 「跑了一阵子的文件传输」提醒（>10s 的后台大件收工），小件与文本不打扰
    pub async fn set_msg_status(&self, id: u64, status: &str) {
        if id == 0 {
            return;
        }
        if let Err(e) = self.db.set_msg_status(id, status) {
            self.log_event(format!("消息 {id} 状态更新失败: {e:#}")).await;
            return;
        }
        if self.panel_watching() {
            return;
        }
        let Some(m) = self.db.get_msg(id) else { return };
        if !m.out {
            return;
        }
        let label = if m.kind == "text" {
            crate::notify::preview(&m.text)
        } else {
            m.name.clone()
        };
        match status {
            "fail" => crate::notify::outgoing(&m.peer_alias, &m.peer, &label, false),
            "ok" if m.kind == "file" && now_ms().saturating_sub(m.at) > 10_000 => {
                crate::notify::outgoing(&m.peer_alias, &m.peer, &label, true)
            }
            _ => {}
        }
    }

    /// 出站图片入缓存；总量超限按最旧（id 最小）淘汰
    pub async fn cache_image(&self, id: u64, mime: String, bytes: Vec<u8>) {
        let mut c = self.img_cache.lock().await;
        let mut total: usize = c.values().map(|i| i.bytes.len()).sum();
        total = total.saturating_add(bytes.len());
        c.insert(id, CachedImg { mime, bytes });
        while total > IMG_CACHE_TOTAL {
            let Some((_, old)) = c.pop_first() else { break };
            total = total.saturating_sub(old.bytes.len());
        }
    }

    /// 下载目录里不冲突的落盘路径（同名自动加 " (n)"）。
    /// 目录读自 RwLock（设置面板改目录后即刻生效）
    pub async fn resolve_path(&self, raw_name: &str) -> PathBuf {
        let safe = raw_name
            .rsplit(['/', '\\'])
            .next()
            .filter(|s| !s.is_empty() && *s != "." && *s != "..")
            .unwrap_or("file");
        let dir = self.download_dir.read().await.clone();
        let mut candidate = dir.join(safe);
        if !candidate.exists() {
            return candidate;
        }
        let p = std::path::Path::new(safe);
        let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("file");
        let ext = p.extension().and_then(|s| s.to_str()).map(|e| format!(".{e}"));
        for n in 1..u32::MAX {
            candidate = dir.join(match &ext {
                Some(e) => format!("{stem} ({n}){e}"),
                None => format!("{stem} ({n})"),
            });
            if !candidate.exists() {
                return candidate;
            }
        }
        dir.join(format!("{}-{}", now_ms(), safe))
    }
}

fn humantime(ms: u64) -> String {
    // CLI 日志用 UTC（带 Z 后缀）；面板端由 JS 按本地时区渲染
    let secs = ms / 1000;
    let rem = secs % 86400;
    format!("{:02}:{:02}:{:02}Z", rem / 3600, (rem % 3600) / 60, rem % 60)
}
