//! 断点续传暂存区：内容寻址目录 + sidecar 元数据。
//!
//! 结构 `<下载目录>/.antify-incoming/<sha256小写>-<size>/{data, meta.json}`：
//! - data 是**已通过块 hash 验证**的字节前缀，meta.received 是其长度；
//! - 持久化顺序恒为「先 fsync data，再原子写 meta」，掉电后 meta 不会超前于
//!   已落盘数据；即便超前（文件系统重排序），reconcile 的互校也能自愈；
//! - 同名不同内容 → 不同 sha → 不同目录，天然不碰撞；同内容跨会话/跨设备
//!   → 同目录自动续上。
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub const STAGING_DIR: &str = ".antify-incoming";
pub const DATA_FILE: &str = "data";
pub const META_FILE: &str = "meta.json";
pub const META_VERSION: u8 = 1;

/// 一个进行中的分块传输的持久化状态
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PartMeta {
    pub version: u8,
    pub file_name: String,
    pub size: u64,
    /// 小写 hex；内容寻址的键
    pub sha256: String,
    /// data 里已验证的字节数
    pub received: u64,
    pub updated_at: u64,
}

pub fn staging_root(download_dir: &Path) -> PathBuf {
    download_dir.join(STAGING_DIR)
}

/// 内容寻址的传输目录。sha256 归一小写，避免大小写差异分裂成两个目录
pub fn transfer_dir(download_dir: &Path, sha256: &str, size: u64) -> PathBuf {
    staging_root(download_dir).join(format!("{}-{}", sha256.to_ascii_lowercase(), size))
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    hex::encode(h.finalize())
}

/// 读 meta；缺失/损坏一律 None（损坏时上层按 received=0 重传——宁重传不交付坏数据）
pub fn read_meta(dir: &Path) -> Option<PartMeta> {
    let raw = std::fs::read_to_string(dir.join(META_FILE)).ok()?;
    let m: PartMeta = serde_json::from_str(&raw).ok()?;
    // 版本不认识 / 键值非法 → 当损坏处理
    (m.version == META_VERSION && m.received <= m.size).then_some(m)
}

/// 原子写 meta：唯一 tmp → fsync → rename（Windows 的 rename 不能覆盖已存在，先删旧）。
/// 调用前必须已 fsync 完 data —— meta.received 只许指向已持久化的数据
pub fn write_meta_atomic(dir: &Path, m: &PartMeta) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!("meta.{}.tmp", uuid::Uuid::new_v4()));
    {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(serde_json::to_string(m).unwrap_or_default().as_bytes())?;
        f.write_all(b"\n")?;
        f.sync_all()?;
    }
    let dst = dir.join(META_FILE);
    #[cfg(target_os = "windows")]
    let _ = std::fs::remove_file(&dst); // rename 不可覆盖已存在
    std::fs::rename(&tmp, &dst)
}

/// 打开 data 前的自愈互校：
/// - data 比 received 长（上次块写一半崩溃）→ 截掉未验证尾巴；
/// - data 比 received 短（meta 超前）或 meta 缺失/损坏 → received 回落到 data 长度；
/// - received 超 size（防御）→ 整体归零重传。
/// 有变化则回写 meta。目录不存在 → 返回 received=0 的 fallback，不落盘
pub async fn reconcile(dir: &Path, fallback: &PartMeta) -> std::io::Result<PartMeta> {
    if !dir.is_dir() {
        let mut m = fallback.clone();
        m.received = 0;
        return Ok(m);
    }
    let stored = read_meta(dir);
    let mut meta = stored.clone().unwrap_or_else(|| {
        let mut m = fallback.clone();
        m.received = 0;
        m.updated_at = crate::state::now_ms();
        m
    });
    let data_path = dir.join(DATA_FILE);
    let data_len = tokio::fs::metadata(&data_path).await.map(|m| m.len()).unwrap_or(0);

    // meta 缺失/损坏 → 重建；data/received 失衡 → 修正。有变化就回写
    let mut dirty = stored.is_none();
    if meta.received > meta.size {
        // 数值自相矛盾：全部作废
        meta.received = 0;
        dirty = true;
        if data_len > 0 {
            truncate(&data_path, 0).await?;
        }
    } else if data_len > meta.received {
        truncate(&data_path, meta.received).await?;
        dirty = true;
    } else if data_len < meta.received {
        meta.received = data_len;
        dirty = true;
    }
    if dirty {
        meta.updated_at = crate::state::now_ms();
        write_meta_atomic(dir, &meta)?;
    }
    Ok(meta)
}

async fn truncate(path: &Path, len: u64) -> std::io::Result<()> {
    let f = tokio::fs::File::options().read(true).write(true).open(path).await?;
    f.set_len(len).await
}

/// 清扫过期暂存目录（活跃传输的 updated_at 恒新，按龄清扫天然安全）。
/// 目录年龄取 meta.updated_at，无 meta 用目录 mtime；再兜底删掉空的 tmp 残件
pub async fn sweep_orphans(state: &crate::state::Shared, max_age: std::time::Duration) {
    let root = staging_root(&*state.download_dir.read().await);
    let Ok(entries) = std::fs::read_dir(&root) else {
        return;
    };
    let cutoff = crate::state::now_ms().saturating_sub(max_age.as_millis() as u64);
    let mut removed = 0u32;
    let mut freed: u64 = 0;
    for e in entries.flatten() {
        let p = e.path();
        let age_ms = read_meta(&p)
            .map(|m| m.updated_at)
            .or_else(|| {
                e.metadata()
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_millis() as u64)
            });
        let Some(age) = age_ms else { continue };
        if age >= cutoff {
            continue;
        }
        let size = dir_size(&p);
        if std::fs::remove_dir_all(&p).is_ok() {
            removed += 1;
            freed += size;
        }
    }
    // 兜底：清掉散落在暂存根目录的 meta.*.tmp 残件（write 中途崩溃留下）
    if let Ok(entries) = std::fs::read_dir(&root) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with("meta.") && name.ends_with(".tmp") {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    if removed > 0 {
        state
            .log_event(format!("清扫了 {removed} 个过期未完成的暂存传输（约 {}）", humantize(freed)))
            .await;
    }
}

fn dir_size(dir: &Path) -> u64 {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| e.metadata().ok().map(|m| m.len()))
                .sum()
        })
        .unwrap_or(0)
}

fn humantize(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = bytes as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{bytes} B")
    } else {
        format!("{v:.1} {}", UNITS[i])
    }
}

// ═══════════════ 单测 ═══════════════

/// 独立测试目录（cargo test 并行跑，各用例唯一目录）
#[cfg(test)]
fn tmp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("antify-resume-test-{tag}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[cfg(test)]
fn meta_of(received: u64) -> PartMeta {
    PartMeta {
        version: META_VERSION,
        file_name: "a.iso".into(),
        size: 100,
        sha256: "ABCD".into(), // 大写：验证 transfer_dir 归一
        received,
        updated_at: 42,
    }
}

#[test]
fn transfer_dir_normalizes_case() {
    let a = transfer_dir(Path::new("/dl"), "AbCd12", 7);
    let b = transfer_dir(Path::new("/dl"), "abcd12", 7);
    assert_eq!(a, b);
    assert!(a.ends_with("abcd12-7"));
}

#[tokio::test]
async fn meta_roundtrip() {
    let dir = tmp_dir("roundtrip");
    let m = meta_of(30);
    write_meta_atomic(&dir, &m).unwrap();
    assert_eq!(read_meta(&dir).as_ref(), Some(&m));
    // 再写一遍（覆盖旧 meta，含 Windows 先删路径）
    let m2 = PartMeta { received: 60, ..m.clone() };
    write_meta_atomic(&dir, &m2).unwrap();
    assert_eq!(read_meta(&dir).as_ref(), Some(&m2));
}

#[tokio::test]
async fn reconcile_matrix() {
    // 目录不存在 → received=0，不创建任何文件
    let dir = tmp_dir("missing").join("nope");
    let m = reconcile(&dir, &meta_of(30)).await.unwrap();
    assert_eq!(m.received, 0);
    assert!(!dir.exists());

    // data 长 → 截到 received
    let dir = tmp_dir("tail");
    write_meta_atomic(&dir, &meta_of(30)).unwrap();
    std::fs::write(dir.join(DATA_FILE), vec![0u8; 50]).unwrap();
    let m = reconcile(&dir, &meta_of(0)).await.unwrap();
    assert_eq!(m.received, 30);
    assert_eq!(std::fs::metadata(dir.join(DATA_FILE)).unwrap().len(), 30);

    // data 短 → received 回落并回写
    let dir = tmp_dir("short");
    write_meta_atomic(&dir, &meta_of(30)).unwrap();
    std::fs::write(dir.join(DATA_FILE), vec![0u8; 10]).unwrap();
    let m = reconcile(&dir, &meta_of(0)).await.unwrap();
    assert_eq!(m.received, 10);
    assert_eq!(read_meta(&dir).unwrap().received, 10);

    // meta 损坏 → 无从得知哪些字节已验证，安全归零（原子写保证这近乎不会发生）
    let dir = tmp_dir("corrupt");
    write_meta_atomic(&dir, &meta_of(30)).unwrap();
    std::fs::write(dir.join(DATA_FILE), vec![0u8; 20]).unwrap();
    std::fs::write(dir.join(META_FILE), "{oops").unwrap();
    let m = reconcile(&dir, &meta_of(0)).await.unwrap();
    assert_eq!(m.received, 0);
    assert_eq!(std::fs::metadata(dir.join(DATA_FILE)).unwrap().len(), 0);
    assert_eq!(read_meta(&dir).unwrap().received, 0);

    // received > size（防御）→ 归零 + data 清空
    let dir = tmp_dir("overflow");
    let bad = PartMeta { received: 999, ..meta_of(0) };
    write_meta_atomic(&dir, &bad).unwrap();
    std::fs::write(dir.join(DATA_FILE), vec![0u8; 50]).unwrap();
    let m = reconcile(&dir, &meta_of(0)).await.unwrap();
    assert_eq!(m.received, 0);
    assert_eq!(std::fs::metadata(dir.join(DATA_FILE)).unwrap().len(), 0);
}

#[test]
fn sha256_known_vector() {
    assert_eq!(
        sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}
