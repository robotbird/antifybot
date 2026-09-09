//! 身份与持久化：别名 / 端口 / 自签证书（指纹 = 证书 SHA-256，跨重启稳定）
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::net::Ipv4Addr;
use std::path::PathBuf;

/// LocalSend v2 协议常量
pub const PROTOCOL_VERSION: &str = "2.1";
pub const MULTICAST_GROUP: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 167);
pub const DEFAULT_PORT: u16 = 53317;

#[derive(Serialize, Deserialize)]
struct ConfigFile {
    alias: String,
    /// 设置面板改过的保存目录（无此字段 = 仍是默认 ~/Downloads/AntifyBot）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    download_dir: Option<String>,
}

pub struct Identity {
    pub alias: String,
    pub port: u16,
    pub fingerprint: String, // 证书 SHA-256（大写 hex）—— HTTPS 模式下的设备指纹
    pub device_model: String,
    pub download_dir: PathBuf,
    pub cert_pem: PathBuf,
    pub key_pem: PathBuf,
    /// 配置目录（~/.antifybot-rs/）：chat.db 也放这里
    pub cfg_dir: PathBuf,
}

impl Identity {
    /// 加载（或首次生成）身份：`~/.antifybot-rs/` 下 config.json + cert.pem + key.pem。
    /// `persist_alias=false` 时（一次性命令 discover/send）只读不写，避免覆盖常驻节点的别名。
    pub fn load(alias: Option<String>, port: u16, dir: Option<PathBuf>) -> Result<Self> {
        Self::load_opts(alias, port, dir, true)
    }

    pub fn load_opts(alias: Option<String>, port: u16, dir: Option<PathBuf>, persist_alias: bool) -> Result<Self> {
        let home = dirs::home_dir().context("无法定位用户主目录")?;
        let cfg_dir = home.join(".antifybot-rs");
        std::fs::create_dir_all(&cfg_dir).with_context(|| format!("创建 {}", cfg_dir.display()))?;

        let cfg_path = cfg_dir.join("config.json");
        let mut saved_alias = None;
        let mut saved_dir = None;
        if let Ok(text) = std::fs::read_to_string(&cfg_path) {
            if let Ok(c) = serde_json::from_str::<ConfigFile>(&text) {
                // 旧版默认别名带 " 🐜" 后缀，读取时剥掉（与保存值不同即触发回写迁移）
                saved_alias = Some(c.alias.trim_end_matches(" 🐜").to_string());
                saved_dir = c.download_dir.filter(|s| !s.trim().is_empty());
            }
        }
        let alias = alias
            .or(saved_alias.clone())
            .unwrap_or_else(default_alias);
        if persist_alias && saved_alias.as_deref() != Some(alias.as_str()) {
            // 写别名时保留已保存的 download_dir 字段
            let _ = std::fs::write(
                &cfg_path,
                serde_json::to_string_pretty(&ConfigFile {
                    alias: alias.clone(),
                    download_dir: saved_dir.clone(),
                })
                .unwrap(),
            );
        }

        let cert_pem = cfg_dir.join("cert.pem");
        let key_pem = cfg_dir.join("key.pem");
        let fingerprint = match (
            std::fs::read(&cert_pem).ok(),
            std::fs::read(&key_pem).ok(),
        ) {
            (Some(cert), Some(key)) if !cert.is_empty() && !key.is_empty() => {
                cert_fingerprint(&cert)?
            }
            _ => {
                // 证书 SAN 是 IA5String（仅 ASCII），emoji 只留在协议别名里
                let ascii_name: String = alias
                    .chars()
                    .filter(|c| c.is_ascii_alphanumeric() || "-_.".contains(*c))
                    .collect();
                let certified = rcgen::generate_simple_self_signed(vec![
                    if ascii_name.is_empty() { "antify".to_string() } else { ascii_name },
                    "localhost".to_string(),
                ])
                .context("生成自签证书失败")?;
                std::fs::write(&cert_pem, certified.cert.pem())?;
                std::fs::write(&key_pem, certified.key_pair.serialize_pem())?;
                cert_fingerprint(certified.cert.pem().as_bytes())?
            }
        };

        // 启动优先级：显式 --dir > 设置面板保存过的目录 > 默认 ~/Downloads/AntifyBot
        let download_dir = dir
            .or_else(|| saved_dir.map(PathBuf::from))
            .unwrap_or_else(|| home.join("Downloads").join("AntifyBot"));
        std::fs::create_dir_all(&download_dir).ok(); // 失败留待接收时再报

        Ok(Self {
            alias,
            port,
            fingerprint,
            device_model: format!("{} ({})", pretty_os(), "antify-rs"),
            download_dir,
            cert_pem,
            key_pem,
            cfg_dir,
        })
    }

    /// 对外公告的设备信息（多播消息 / register 请求体，含 port/protocol）
    pub fn announce_json(&self, announce: bool) -> serde_json::Value {
        serde_json::json!({
            "announce": announce,
            "alias": self.alias,
            "version": PROTOCOL_VERSION,
            "deviceModel": self.device_model,
            "deviceType": "headless",
            "fingerprint": self.fingerprint,
            "port": self.port,
            "protocol": "https",
            "download": false,
        })
    }

    /// register 请求体（v2 DTO，无 announce 字段）
    pub fn register_json(&self) -> serde_json::Value {
        serde_json::json!({
            "alias": self.alias,
            "version": PROTOCOL_VERSION,
            "deviceModel": self.device_model,
            "deviceType": "headless",
            "fingerprint": self.fingerprint,
            "port": self.port,
            "protocol": "https",
            "download": false,
        })
    }
}

/// 设置面板改保存目录：整写 config.json（保留别名等既有字段）
pub fn save_download_dir(dir: &std::path::Path) -> Result<()> {
    let home = dirs::home_dir().context("无法定位用户主目录")?;
    let cfg_path = home.join(".antifybot-rs").join("config.json");
    let mut cfg = std::fs::read_to_string(&cfg_path)
        .ok()
        .and_then(|t| serde_json::from_str::<ConfigFile>(&t).ok())
        .unwrap_or_else(|| ConfigFile {
            alias: default_alias(),
            download_dir: None,
        });
    cfg.download_dir = Some(dir.to_string_lossy().to_string());
    std::fs::write(&cfg_path, serde_json::to_string_pretty(&cfg).unwrap())
        .with_context(|| format!("写 {}", cfg_path.display()))
}

fn cert_fingerprint(pem: &[u8]) -> Result<String> {
    use base64::Engine;
    let text = std::str::from_utf8(pem).context("证书 PEM 编码异常")?;
    let b64: String = text
        .lines()
        .filter(|l| !l.starts_with("---"))
        .collect::<String>()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    let der = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .context("解析证书 base64 失败")?;
    Ok(hex::encode_upper(Sha256::digest(&der)))
}

fn default_alias() -> String {
    // 优先系统「电脑名称」（macOS ComputerName，如「叶鹏的MacBook Air」），比
    // mDNS 主机名（xiepengdeMacBook-Air）更贴近用户认知；取不到回退主机名
    if let Some(pretty) = os_pretty_name() {
        let t = pretty.trim();
        if !t.is_empty() {
            return t.to_string();
        }
    }
    let host = gethostname::gethostname()
        .to_str()
        .map(|s| s.to_string())
        .unwrap_or_else(|| "Antify".to_string());
    host.split('.').next().unwrap_or("Antify").to_string()
}

/// 系统级设备名：macOS 走 `scutil --get ComputerName`；Windows 取 %COMPUTERNAME%；
/// 其余平台返回 None（调用方回退主机名）
fn os_pretty_name() -> Option<String> {
    if cfg!(target_os = "macos") {
        let out = std::process::Command::new("scutil")
            .args(["--get", "ComputerName"])
            .output()
            .ok()?;
        if out.status.success() {
            Some(String::from_utf8_lossy(&out.stdout).into_owned())
        } else {
            None
        }
    } else if cfg!(target_os = "windows") {
        std::env::var("COMPUTERNAME").ok()
    } else {
        None
    }
}

fn pretty_os() -> &'static str {
    match std::env::consts::OS {
        "macos" => "macOS",
        "windows" => "Windows",
        "linux" => "Linux",
        other => other,
    }
}
