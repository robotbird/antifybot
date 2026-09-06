//! 共享状态：发现的设备、进行中的接收会话、已收文件、事件流
use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
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
    pub attempts: u8,
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

/// 会话流里的一条消息（右侧聊天视图的数据源；仅内存，重启即清）
#[derive(Clone, Serialize)]
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
}

pub struct AppState {
    pub identity: crate::config::Identity,
    pub devices: Mutex<HashMap<String, Device>>,
    pub session: Mutex<Option<Session>>,
    pub received: Mutex<Vec<ReceivedFile>>,
    pub events: Mutex<VecDeque<(u64, String)>>,
    pub sending: Mutex<SendProgress>,
    /// 会话流（右侧聊天视图）：每个设备一条线程，按 peer 过滤
    pub chat: Mutex<Vec<ChatMsg>>,
    /// 出站图片回显缓存（chat 消息 id → 图）：出站文件不落盘，图片留内存供面板预览；
    /// 接收图片在下载目录里，直接读盘不进缓存
    pub img_cache: Mutex<std::collections::BTreeMap<u64, CachedImg>>,
    /// 会话消息自增序号
    pub chat_seq: AtomicU64,
    /// 面板中转发送的实时字节计数（SendProgress 里的是快照，此处原子累加）
    pub relay_bytes: AtomicU64,
    /// 接收侧当前文件进度（upload 处理器实时更新）
    pub rx_bytes: AtomicU64,
    pub rx_total: AtomicU64,
    pub rx_name: Mutex<String>,
}

pub type Shared = Arc<AppState>;

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

impl AppState {
    pub async fn log_event(&self, text: impl Into<String>) {
        let line = text.into();
        println!("[{}] {}", humantime(now_ms()), line);
        let mut ev = self.events.lock().await;
        ev.push_back((now_ms(), line));
        while ev.len() > 200 {
            ev.pop_front();
        }
    }

    /// 会话流追加一条（内存环形，仅保留最近 500 条）；返回消息 id（出站图片回显缓存以此为主键）
    pub async fn push_chat(&self, mut m: ChatMsg) -> u64 {
        m.id = self.chat_seq.fetch_add(1, Ordering::Relaxed);
        let id = m.id;
        let mut c = self.chat.lock().await;
        c.push(m);
        let overflow = c.len().saturating_sub(500);
        if overflow > 0 {
            c.drain(..overflow);
        }
        id
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

    /// 下载目录里不冲突的落盘路径（同名自动加 " (n)"）
    pub fn resolve_path(&self, raw_name: &str) -> PathBuf {
        let safe = raw_name
            .rsplit(['/', '\\'])
            .next()
            .filter(|s| !s.is_empty() && *s != "." && *s != "..")
            .unwrap_or("file");
        let dir = &self.identity.download_dir;
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
