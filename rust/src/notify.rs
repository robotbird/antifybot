//! 系统桌面通知：消息 / 文件到达与出站成败的横幅提醒（GUI 托盘后台与 `serve` CLI 共用）。
//! 每次一条系统命令拉起，独立线程执行、失败静默——提醒是锦上添花，绝不阻塞收发路径：
//! - macOS：`osascript display notification`（通知来源显示为系统脚本，标题带对方别名）
//! - Windows：PowerShell WinRT Toast（`-EncodedCommand` 传 UTF-16LE base64，绕开引号转义；
//!   AUMID 借用系统注册的 Windows PowerShell 项——自造 AUMID 未注册，Win11 会直接丢弃）
//! - Linux：`notify-send`（装了才有效，缺失静默）
//! 文案刻意做成「标题 = 对方别名，正文 = 内容」的中性结构，不夹动词，不受界面语言影响。
use std::time::{Duration, Instant};

/// 突发抑制：同一 key 3 秒内只弹一次（整夹接收 / 批量送达不打横幅轰炸）
static LAST: std::sync::Mutex<Option<(String, Instant)>> = std::sync::Mutex::new(None);

fn throttled(key: &str) -> bool {
    let mut g = LAST.lock().unwrap_or_else(|e| e.into_inner());
    match g.as_ref() {
        Some((k, at)) if k == key && at.elapsed() < Duration::from_secs(3) => true,
        _ => {
            *g = Some((key.to_string(), Instant::now()));
            false
        }
    }
}

/// 正文预览：首个非空行，截 80 字符
pub fn preview(s: &str) -> String {
    s.lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .chars()
        .take(80)
        .collect()
}

fn human(n: u64) -> String {
    const U: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < 4 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", U[i])
    }
}

fn fallback_alias(alias: &str) -> String {
    let t = alias.trim();
    if t.is_empty() { "AntifyBot".into() } else { t.to_string() }
}

/// 收到文字消息（每次都提醒——打字是有意为之，不算突发）
pub fn message(alias: &str, text: &str) {
    show(&fallback_alias(alias), &preview(text));
}

/// 收到文件（key 取发送方指纹：同一发送方 3 秒内连发只提醒第一次）
pub fn file(alias: &str, key: &str, name: &str, size: u64) {
    if throttled(&format!("in:{key}")) {
        return;
    }
    show(&fallback_alias(alias), &format!("📄 {name} · {}", human(size)));
}

/// 出站流转：ok 只在突发抑制内放行第一条；失败总是提醒
pub fn outgoing(alias: &str, key: &str, label: &str, ok: bool) {
    if ok && throttled(&format!("out:{key}")) {
        return;
    }
    let mark = if ok { "✓" } else { "⚠" };
    show(
        &fallback_alias(alias),
        &format!("{mark} {}", preview(label)),
    );
}

fn show(title: &str, body: &str) {
    let (t, b) = (title.to_string(), body.to_string());
    std::thread::spawn(move || {
        #[cfg(target_os = "macos")]
        let out = std::process::Command::new("osascript")
            .arg("-e")
            .arg(format!(
                "display notification {} with title {}",
                applescript(&b),
                applescript(&t)
            ))
            .output();

        #[cfg(target_os = "windows")]
        let out = toast_windows(&t, &b);

        #[cfg(all(unix, not(target_os = "macos")))]
        let out = std::process::Command::new("notify-send")
            .arg(&t)
            .arg(&b)
            .output();

        #[cfg(not(any(unix, target_os = "windows")))]
        let out: std::io::Result<std::process::Output> = Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "此平台无通知实现",
        ));

        let _ = out; // 失败静默：通知不可用不影响收发功能
    });
}

/// AppleScript 字符串字面量（反斜杠与双引号转义）
#[cfg(target_os = "macos")]
fn applescript(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Windows Toast：XML 转义 + PowerShell 单引号加倍后装进单引号串，经
/// -EncodedCommand 投递；CREATE_NO_WINDOW 避免每次弹横幅闪一下控制台
#[cfg(target_os = "windows")]
fn toast_windows(title: &str, body: &str) -> std::io::Result<std::process::Output> {
    use base64::Engine;
    use std::os::windows::process::CommandExt;
    let esc = |s: &str| {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('\'', "''")
    };
    let ps = format!(
        "$null=[Windows.UI.Notifications.ToastNotificationManager,Windows.UI.Notifications,ContentType=WindowsRuntime];\
         $null=[Windows.Data.Xml.Dom.XmlDocument,Windows.Data.Xml.Dom.XmlDocument,ContentType=WindowsRuntime];\
         $x=New-Object Windows.Data.Xml.Dom.XmlDocument;\
         $x.LoadXml('<toast><visual><binding template=\"ToastGeneric\"><text>{}</text><text>{}</text></binding></visual></toast>');\
         $t=New-Object Windows.UI.Notifications.ToastNotification $x;\
         [Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier('{{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}}\\WindowsPowerShell\\v1.0\\powershell.exe').Show($t)",
        esc(title),
        esc(body)
    );
    let mut b = Vec::new();
    for u in ps.encode_utf16() {
        b.extend_from_slice(&u.to_le_bytes());
    }
    std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-EncodedCommand",
            &base64::engine::general_purpose::STANDARD.encode(b),
        ])
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .output()
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    /// 真弹一条：引号 / 反斜杠 / 换行 / emoji 混合内容必须被 AppleScript 接受
    #[test]
    fn osascript_accepts_escaped() {
        let script = format!(
            "display notification {} with title {}",
            applescript("a\"b\\c\nd🐜"),
            applescript("标\"题")
        );
        let out = std::process::Command::new("osascript")
            .arg("-e")
            .arg(&script)
            .output()
            .expect("osascript");
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    }

    #[test]
    fn human_sizes() {
        assert_eq!(human(0), "0 B");
        assert_eq!(human(512), "512 B");
        assert_eq!(human(3 * 1024 * 1024), "3.0 MB");
        assert_eq!(human(10 * 1024_u64.pow(3)), "10.0 GB");
    }

    #[test]
    fn preview_takes_first_line() {
        assert_eq!(preview("  \n你好\n第二行"), "你好");
        assert_eq!(preview(&"x".repeat(200)).chars().count(), 80);
    }
}
