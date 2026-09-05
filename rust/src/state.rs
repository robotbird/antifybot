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
    pub done: bool,
    pub attempts: u8,
}

/// 活动接收会话（协议约束：同时仅一个）
pub struct Session {
    pub id: String,
    pub sender_ip: IpAddr,
    pub sender_alias: String,
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

pub struct AppState {
    pub identity: crate::config::Identity,
    pub devices: Mutex<HashMap<String, Device>>,
    pub session: Mutex<Option<Session>>,
    pub received: Mutex<Vec<ReceivedFile>>,
    pub events: Mutex<VecDeque<(u64, String)>>,
    pub sending: Mutex<SendProgress>,
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
