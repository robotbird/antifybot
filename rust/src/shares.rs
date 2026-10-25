//! Read-only capability shares. Only panel endpoints can publish/revoke or write downloads.
//! Network endpoints never accept absolute paths and never follow child symlinks.
use crate::state::Shared;
use anyhow::{bail, ensure, Context, Result};
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, UNIX_EPOCH},
};
use tokio::{
    io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt},
    sync::Mutex,
};

const CHUNK: u64 = 4 * 1024 * 1024;
#[derive(Default)]
pub struct PullState {
    lock: Mutex<()>,
    progress: Mutex<Progress>,
    cancelled: AtomicBool,
}
#[derive(Default, Serialize)]
struct Progress {
    active: bool,
    path: String,
    received: u64,
    total: u64,
    error: String,
}
#[derive(Serialize, Deserialize)]
pub(crate) struct Share {
    pub(crate) token: String,
    pub(crate) name: String,
    pub(crate) root: PathBuf,
}
#[derive(Deserialize)]
pub struct Publish {
    path: String,
}
#[derive(Deserialize)]
pub struct Token {
    token: String,
}
#[derive(Clone, Deserialize, Serialize)]
pub struct Location {
    token: String,
    #[serde(default)]
    path: String,
}
#[derive(Deserialize)]
pub struct Browse {
    target: String,
    token: String,
    #[serde(default)]
    path: String,
    #[serde(default)]
    cursor: String,
}
#[derive(Deserialize)]
pub struct Listing {
    token: String,
    #[serde(default)]
    path: String,
    #[serde(default)]
    cursor: String,
}
#[derive(Deserialize)]
pub struct Range {
    token: String,
    path: String,
    version: String,
    #[serde(default)]
    offset: u64,
}
#[derive(Deserialize, Serialize)]
pub(crate) struct Manifest {
    pub(crate) size: u64,
    pub(crate) sha256: String,
    pub(crate) version: String,
}

pub(crate) fn reply(result: Result<serde_json::Value>) -> Response {
    match result {
        Ok(v) => Json(v).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": format!("{e:#}")})),
        )
            .into_response(),
    }
}
pub(crate) fn valid_token(token: &str) -> Result<()> {
    ensure!(
        token.len() == 32 && token.bytes().all(|b| b.is_ascii_hexdigit()),
        "无效共享地址"
    );
    Ok(())
}
pub(crate) fn parts(path: &str) -> Result<Vec<&str>> {
    if path.is_empty() {
        return Ok(vec![]);
    }
    let p: Vec<_> = path.split('/').collect();
    ensure!(
        p.iter().all(|s| !s.is_empty()
            && *s != "."
            && *s != ".."
            && !s.contains(['\\', ':', '\0'])
            && !s.ends_with(['.', ' '])
            && !s.chars().any(char::is_control)),
        "路径必须是安全的相对路径"
    );
    Ok(p)
}
fn store(state: &Shared) -> PathBuf {
    state.identity.cfg_dir.join("shares")
}
pub(crate) async fn load(state: &Shared, token: &str) -> Result<Share> {
    valid_token(token)?;
    let bytes = tokio::fs::read(store(state).join(format!("{token}.json")))
        .await
        .context("共享不存在或已撤销")?;
    Ok(serde_json::from_slice(&bytes)?)
}
pub(crate) async fn resolve(root: &Path, path: &str) -> Result<PathBuf> {
    let mut out = root.to_owned();
    ensure!(
        tokio::fs::canonicalize(root).await? == root,
        "共享根目录已改变"
    );
    for part in parts(path)? {
        out.push(part);
        let m = tokio::fs::symlink_metadata(&out).await?;
        ensure!(!m.file_type().is_symlink(), "不允许访问符号链接");
    }
    Ok(out)
}
async fn file(state: &Shared, loc: &Location) -> Result<PathBuf> {
    let share = load(state, &loc.token).await?;
    let path = resolve(&share.root, &loc.path).await?;
    ensure!(tokio::fs::metadata(&path).await?.is_file(), "不是常规文件");
    Ok(path)
}
pub(crate) fn version(m: &std::fs::Metadata) -> Result<String> {
    Ok(format!(
        "{}-{}",
        m.len(),
        m.modified()?.duration_since(UNIX_EPOCH)?.as_nanos()
    ))
}
pub(crate) async fn hash(path: &Path) -> Result<String> {
    let mut f = tokio::fs::File::open(path).await?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = f.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}
pub async fn publish(State(state): State<Shared>, Json(body): Json<Publish>) -> Response {
    reply(async {
        let root = tokio::fs::canonicalize(body.path).await?;
        ensure!(tokio::fs::metadata(&root).await?.is_dir(), "请选择文件夹");
        let token = uuid::Uuid::new_v4().simple().to_string();
        let name = root.file_name().unwrap_or_default().to_string_lossy().to_string();
        let share = Share { token: token.clone(), name, root };
        tokio::fs::create_dir_all(store(&state)).await?;
        let mut f = tokio::fs::OpenOptions::new().write(true).create_new(true)
            .open(store(&state).join(format!("{token}.json"))).await?;
        f.write_all(&serde_json::to_vec(&share)?).await?;
        f.sync_all().await?;
        Ok(json!({"token": token, "name": share.name, "address": format!("antify-share://{}/{}", state.identity.fingerprint, token)}))
    }.await)
}
pub async fn local_list(State(state): State<Shared>) -> Response {
    reply(
        async {
            tokio::fs::create_dir_all(store(&state)).await?;
            let mut rd = tokio::fs::read_dir(store(&state)).await?;
            let mut shares = vec![];
            while let Some(e) = rd.next_entry().await? {
                if e.path().extension().and_then(|x| x.to_str()) != Some("json") { continue; }
                if let Ok(s) = serde_json::from_slice::<Share>(&tokio::fs::read(e.path()).await?) {
                    shares.push(json!({"token":s.token,"name":s.name,"path":s.root}));
                }
            }
            Ok(json!({"shares":shares}))
        }
        .await,
    )
}
pub async fn revoke(State(state): State<Shared>, Json(body): Json<Token>) -> Response {
    reply(
        async {
            valid_token(&body.token)?;
            tokio::fs::remove_file(store(&state).join(format!("{}.json", body.token))).await?;
            Ok(json!({"ok":true}))
        }
        .await,
    )
}

pub async fn list(State(state): State<Shared>, Query(q): Query<Listing>) -> Response {
    reply(async {
        let share = load(&state, &q.token).await?;
        let dir = resolve(&share.root, &q.path).await?;
        let mut rd = tokio::fs::read_dir(dir).await?;
        // Bounded memory even for million-entry directories: lexical cursor, 200 per page.
        let mut entries = std::collections::BTreeMap::new();
        while let Some(e) = rd.next_entry().await? {
            let Ok(name) = e.file_name().into_string() else { continue; };
            if name <= q.cursor || parts(&name).is_err() { continue; }
            if entries.len() == 201 && entries.keys().next_back().is_some_and(|last| &name > last) { continue; }
            let m = tokio::fs::symlink_metadata(e.path()).await?;
            if !m.is_file() && !m.is_dir() { continue; }
            entries.insert(name.clone(), json!({"name":name,"directory":m.is_dir(),"size":m.len()}));
            if entries.len() > 201 { entries.pop_last(); }
        }
        let more = entries.len() > 200;
        if more { entries.pop_last(); }
        let next = if more { entries.keys().next_back().cloned() } else { None };
        Ok(json!({"name":share.name,"entries":entries.into_values().collect::<Vec<_>>(),"next":next}))
    }.await)
}
pub async fn manifest(State(state): State<Shared>, Query(q): Query<Location>) -> Response {
    reply(
        async {
            let path = file(&state, &q).await?;
            let before = tokio::fs::metadata(&path).await?;
            let v = version(&before)?;
            let sha256 = hash(&path).await?;
            ensure!(
                version(&tokio::fs::metadata(&path).await?)? == v,
                "文件正在改变，请稍后重试"
            );
            Ok(serde_json::to_value(Manifest {
                size: before.len(),
                sha256,
                version: v,
            })?)
        }
        .await,
    )
}
pub async fn range(State(state): State<Shared>, Query(q): Query<Range>) -> Response {
    let result: Result<Response> = async {
        let path = file(
            &state,
            &Location {
                token: q.token,
                path: q.path,
            },
        )
        .await?;
        let mut f = tokio::fs::File::open(&path).await?;
        let m = f.metadata().await?;
        ensure!(version(&m)? == q.version, "源文件已改变，请重新下载");
        ensure!(q.offset < m.len(), "偏移超出文件范围");
        let n = CHUNK.min(m.len() - q.offset) as usize;
        let mut bytes = vec![0u8; n];
        f.seek(std::io::SeekFrom::Start(q.offset)).await?;
        f.read_exact(&mut bytes).await?;
        ensure!(
            version(&f.metadata().await?)? == q.version,
            "源文件正在改变"
        );
        let digest = hex::encode(Sha256::digest(&bytes));
        Ok((
            StatusCode::PARTIAL_CONTENT,
            [
                ("content-type", "application/octet-stream".to_string()),
                (
                    "content-range",
                    format!("bytes {}-{}/{}", q.offset, q.offset + n as u64 - 1, m.len()),
                ),
                ("x-antify-sha256", digest),
                ("cache-control", "no-store".to_string()),
            ],
            bytes,
        )
            .into_response())
    }
    .await;
    match result {
        Ok(r) => r,
        Err(e) => reply(Err(e)),
    }
}

// Remote requests are limited to discovered peers, never arbitrary panel-supplied URLs.
pub(crate) async fn remote(state: &Shared, target: &str, token: &str) -> Result<(reqwest::Client, String)> {
    valid_token(token)?;
    let d = state
        .devices
        .lock()
        .await
        .get(target)
        .cloned()
        .context("设备不在列表中，请等待发现设备")?;
    ensure!(d.https, "文件夹共享只支持 HTTPS 节点");
    // Pin the actual TLS certificate, not the unauthenticated discovery payload.
    let expected = target.to_ascii_lowercase();
    let client = reqwest::Client::builder()
        .use_rustls_tls()
        .danger_accept_invalid_certs(true)
        .tls_info(true)
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .connect_timeout(Duration::from_secs(5))
        .build()?;
    let response = client
        .get(format!("{}/api/localsend/v2/info", d.base_url()))
        .timeout(Duration::from_secs(10))
        .send()
        .await?;
    let cert = response
        .extensions()
        .get::<reqwest::tls::TlsInfo>()
        .and_then(|i| i.peer_certificate())
        .context("未获取到对端证书")?;
    ensure!(
        hex::encode(Sha256::digest(cert)) == expected,
        "设备证书指纹不匹配"
    );
    let pinned = reqwest::Client::builder()
        .use_rustls_tls()
        .tls_built_in_root_certs(false)
        .add_root_certificate(reqwest::Certificate::from_der(cert)?)
        .danger_accept_invalid_hostnames(true)
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .connect_timeout(Duration::from_secs(5))
        .build()?;
    Ok((pinned, d.base_url()))
}
pub(crate) async fn remote_json(response: reqwest::Response) -> Result<serde_json::Value> {
    let status = response.status();
    let v: serde_json::Value = response
        .json()
        .await
        .context("对端不支持文件夹共享或响应无效")?;
    ensure!(
        status.is_success(),
        "{}",
        v.get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("远端请求失败")
    );
    Ok(v)
}
pub async fn browse(State(state): State<Shared>, Json(q): Json<Browse>) -> Response {
    reply(
        async {
            parts(&q.path)?;
            let (client, base) = remote(&state, &q.target, &q.token).await?;
            remote_json(
                client
                    .get(format!("{base}/api/antify/v1/share/list"))
                    .query(&[("token", q.token), ("path", q.path), ("cursor", q.cursor)])
                    .timeout(Duration::from_secs(30))
                    .send()
                    .await?,
            )
            .await
        }
        .await,
    )
}

pub(crate) async fn safe_dir(root: &Path, segments: &[&str]) -> Result<PathBuf> {
    let mut dir = root.to_owned();
    for s in segments {
        dir.push(s);
        match tokio::fs::create_dir(&dir).await {
            Ok(()) => (),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
            Err(e) => return Err(e.into()),
        }
        let m = tokio::fs::symlink_metadata(&dir).await?;
        ensure!(
            m.is_dir() && !m.file_type().is_symlink(),
            "下载目录含符号链接或不是目录"
        );
    }
    Ok(dir)
}
async fn download(state: &Shared, q: &Browse) -> Result<serde_json::Value> {
    let segments = parts(&q.path)?;
    ensure!(!segments.is_empty(), "请选择文件");
    let (client, base) = remote(state, &q.target, &q.token).await?;
    let m: Manifest = serde_json::from_value(
        remote_json(
            client
                .get(format!("{base}/api/antify/v1/share/manifest"))
                .query(&[("token", &q.token), ("path", &q.path)])
                .timeout(Duration::from_secs(3600))
                .send()
                .await?,
        )
        .await?,
    )?;
    ensure!(
        m.sha256.len() == 64 && m.sha256.bytes().all(|b| b.is_ascii_hexdigit()),
        "无效文件摘要"
    );
    let root = tokio::fs::canonicalize(state.download_dir.read().await.clone()).await?;
    let folder = format!("Shared-{}", q.token);
    let mut dirs = vec![folder.as_str()];
    dirs.extend_from_slice(&segments[..segments.len() - 1]);
    let dest_dir = safe_dir(&root, &dirs).await?;
    let staging = safe_dir(&root, &[".antify-pulls"]).await?;
    let key = hex::encode(Sha256::digest(serde_json::to_vec(&(
        &q.target, &q.token, &q.path, &m.sha256, m.size,
    ))?));
    let part = staging.join(format!("{key}.part"));
    if let Ok(meta) = tokio::fs::symlink_metadata(&part).await {
        ensure!(
            meta.is_file() && !meta.file_type().is_symlink(),
            "暂存文件不安全"
        );
    }
    let mut out = tokio::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&part)
        .await?;
    let len = out.metadata().await?.len().min(m.size);
    // A crash may leave a partially written chunk: only restart at verified chunk boundaries.
    let mut offset = if len == m.size {
        len
    } else {
        len / CHUNK * CHUNK
    };
    out.set_len(offset).await?;
    out.seek(std::io::SeekFrom::Start(offset)).await?;
    {
        let mut p = state.pulls.progress.lock().await;
        p.total = m.size;
        p.received = offset;
    }
    while offset < m.size {
        ensure!(
            !state.pulls.cancelled.load(Ordering::Relaxed),
            "已暂停，再次下载将从断点继续"
        );
        let mut chunk = None;
        let mut last_error = String::new();
        for attempt in 0..3 {
            let result: Result<Vec<u8>> = async {
                let r = client
                    .get(format!("{base}/api/antify/v1/share/file"))
                    .query(&[
                        ("token", q.token.clone()),
                        ("path", q.path.clone()),
                        ("version", m.version.clone()),
                        ("offset", offset.to_string()),
                    ])
                    .timeout(Duration::from_secs(120))
                    .send()
                    .await?;
                ensure!(
                    r.status() == StatusCode::PARTIAL_CONTENT,
                    "远端拒绝下载（共享撤销或文件改变）"
                );
                let digest = r
                    .headers()
                    .get("x-antify-sha256")
                    .context("缺少分块校验")?
                    .to_str()?
                    .to_owned();
                let expected = CHUNK.min(m.size - offset) as usize;
                let mut stream = r.bytes_stream();
                let mut bytes = Vec::with_capacity(expected);
                use futures_util::StreamExt;
                while let Some(b) = stream.next().await {
                    let b = b?;
                    ensure!(bytes.len() + b.len() <= expected, "分块超出大小");
                    bytes.extend_from_slice(&b);
                }
                ensure!(
                    bytes.len() == expected && hex::encode(Sha256::digest(&bytes)) == digest,
                    "分块校验失败"
                );
                Ok(bytes)
            }
            .await;
            match result {
                Ok(b) => {
                    chunk = Some(b);
                    break;
                }
                Err(e) => last_error = format!("{e:#}"),
            }
            if state.pulls.cancelled.load(Ordering::Relaxed) {
                bail!("已暂停，断点已保留");
            }
            tokio::time::sleep(Duration::from_millis(300 * (attempt + 1))).await;
        }
        let bytes = chunk.context(last_error)?;
        out.write_all(&bytes).await?;
        out.sync_data().await?;
        offset += bytes.len() as u64;
        state.pulls.progress.lock().await.received = offset;
    }
    out.sync_all().await?;
    drop(out);
    if hash(&part).await? != m.sha256 {
        tokio::fs::remove_file(&part).await?;
        bail!("整文件 SHA-256 校验失败，暂存已清除，请重试");
    }
    // hard_link provides atomic no-clobber publication (staging is on the same filesystem).
    let name = segments.last().unwrap();
    let mut destination = dest_dir.join(name);
    for n in 0..10000 {
        destination = if n == 0 {
            dest_dir.join(name)
        } else {
            dest_dir.join(format!("{name} ({n})"))
        };
        match tokio::fs::hard_link(&part, &destination).await {
            Ok(()) => break,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && n < 9999 => continue,
            Err(e) => return Err(e).context("保存下载文件失败"),
        }
    }
    tokio::fs::remove_file(&part).await?;
    let relative = destination
        .strip_prefix(&root)?
        .to_string_lossy()
        .to_string();
    let alias = state
        .devices
        .lock()
        .await
        .get(&q.target)
        .map(|d| d.alias.clone())
        .unwrap_or_default();
    state
        .push_chat(crate::state::ChatMsg {
            out: false,
            peer: q.target.clone(),
            peer_alias: alias,
            kind: "file".into(),
            name: name.to_string(),
            size: m.size,
            file: relative.clone(),
            at: crate::state::now_ms(),
            ..Default::default()
        })
        .await;
    Ok(json!({"ok":true,"file":relative}))
}
pub async fn pull(State(state): State<Shared>, Json(q): Json<Browse>) -> Response {
    let Ok(_guard) = state.pulls.lock.try_lock() else {
        return reply(Err(anyhow::anyhow!("已有文件正在下载")));
    };
    state.pulls.cancelled.store(false, Ordering::Relaxed);
    *state.pulls.progress.lock().await = Progress {
        active: true,
        path: q.path.clone(),
        ..Default::default()
    };
    let result = download(&state, &q).await;
    let mut p = state.pulls.progress.lock().await;
    p.active = false;
    if let Err(e) = &result {
        p.error = format!("{e:#}");
    }
    reply(result)
}
pub async fn progress(State(state): State<Shared>) -> Response {
    Json(json!(*state.pulls.progress.lock().await)).into_response()
}
pub async fn pause(State(state): State<Shared>) -> Response {
    state.pulls.cancelled.store(true, Ordering::Relaxed);
    Json(json!({"ok":true})).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reject_traversal() {
        for path in [
            "/etc/passwd",
            "../secret",
            "a/../../b",
            "a\\b",
            "C:/x",
            "a//b",
            "a/./b",
            "a\0b",
            "a.",
            "a ",
        ] {
            assert!(parts(path).is_err(), "{path}");
        }
        assert!(parts("目录/sub/file.txt").is_ok());
        assert!(parts("").is_ok());
        assert!(valid_token("../../secret").is_err());
    }

    struct Fixture {
        root: PathBuf,
        state: Shared,
    }
    impl Fixture {
        fn new() -> Self {
            crate::install_crypto_provider();
            let root =
                std::env::temp_dir().join(format!("antify-share-test-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(root.join("downloads")).unwrap();
            let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
            std::fs::write(root.join("cert.pem"), cert.cert.pem()).unwrap();
            std::fs::write(root.join("key.pem"), cert.key_pair.serialize_pem()).unwrap();
            let id = crate::config::Identity {
                alias: "test".into(),
                port: crate::server::pick_free_port(54377),
                fingerprint: hex::encode_upper(Sha256::digest(cert.cert.der())),
                device_model: "test".into(),
                download_dir: root.join("downloads"),
                cert_pem: root.join("cert.pem"),
                key_pem: root.join("key.pem"),
                cfg_dir: root.clone(),
            };
            let state = crate::server::new_state(id).unwrap();
            Self { root, state }
        }
        async fn publish(&self, path: &Path) -> String {
            let v = value(
                publish(
                    State(self.state.clone()),
                    Json(Publish {
                        path: path.to_string_lossy().into(),
                    }),
                )
                .await,
            )
            .await;
            v["token"].as_str().unwrap().into()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
    async fn value(response: Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(response.into_body(), 16 * 1024 * 1024)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }
    #[tokio::test]
    async fn pagination_security_and_revoke() {
        let f = Fixture::new();
        let root = f.root.join("source");
        std::fs::create_dir(&root).unwrap();
        for n in 0..1205 {
            std::fs::write(root.join(format!("file-{n:04}")), b"data").unwrap();
        }
        std::fs::create_dir(root.join("nested")).unwrap();
        std::fs::write(root.join("nested/中文.txt"), b"hello").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&f.root, root.join("escape")).unwrap();
        let token = f.publish(&root).await;
        let mut cursor = String::new();
        let mut names = std::collections::BTreeSet::new();
        loop {
            let v = value(
                list(
                    State(f.state.clone()),
                    Query(Listing {
                        token: token.clone(),
                        path: String::new(),
                        cursor,
                    }),
                )
                .await,
            )
            .await;
            let entries = v["entries"].as_array().unwrap();
            assert!(entries.len() <= 200);
            for e in entries {
                assert!(names.insert(e["name"].as_str().unwrap().to_string()));
            }
            match v["next"].as_str() {
                Some(next) => cursor = next.into(),
                None => break,
            }
        }
        assert_eq!(names.len(), 1206);
        assert!(!names.contains("escape"));
        assert!(resolve(&root, "../cert.pem").await.is_err());
        #[cfg(unix)]
        assert!(resolve(&root, "escape/cert.pem").await.is_err());
        let m = value(
            manifest(
                State(f.state.clone()),
                Query(Location {
                    token: token.clone(),
                    path: "nested/中文.txt".into(),
                }),
            )
            .await,
        )
        .await;
        assert_eq!(m["sha256"], hex::encode(Sha256::digest(b"hello")));
        let response = range(
            State(f.state.clone()),
            Query(Range {
                token: token.clone(),
                path: "nested/中文.txt".into(),
                version: "stale".into(),
                offset: 0,
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        // Capability survives a node restart because mapping is on disk.
        assert_eq!(
            load(&f.state, &token).await.unwrap().root,
            tokio::fs::canonicalize(&root).await.unwrap()
        );
        assert!(load(&f.state, &"0".repeat(32)).await.is_err());
        revoke(
            State(f.state.clone()),
            Json(Token {
                token: token.clone(),
            }),
        )
        .await;
        assert!(load(&f.state, &token).await.is_err());
    }
    #[tokio::test]
    async fn tls_pull_resumes_verifies_and_preserves_tree() {
        let f = Fixture::new();
        let source = f.root.join("source");
        std::fs::create_dir_all(source.join("nested")).unwrap();
        let bytes: Vec<_> = (0..(CHUNK as usize * 2 + 123))
            .map(|i| (i % 251) as u8)
            .collect();
        std::fs::write(source.join("nested/large.bin"), &bytes).unwrap();
        std::fs::write(source.join("empty"), b"").unwrap();
        let token = f.publish(&source).await;
        let fp = f.state.identity.fingerprint.clone();
        f.state
            .upsert_device(crate::state::Device {
                fingerprint: fp.clone(),
                alias: "source".into(),
                ip: "127.0.0.1".parse().unwrap(),
                port: f.state.identity.port,
                https: true,
                device_model: None,
                device_type: None,
                version: "2.1".into(),
                download: false,
                last_seen: 0,
            })
            .await;
        let tls = axum_server::tls_rustls::RustlsConfig::from_pem_file(
            &f.state.identity.cert_pem,
            &f.state.identity.key_pem,
        )
        .await
        .unwrap();
        let handle = axum_server::Handle::new();
        let shutdown = handle.clone();
        let addr = ([127, 0, 0, 1], f.state.identity.port).into();
        let offsets = std::sync::Arc::new(Mutex::new(Vec::<u64>::new()));
        let recorded = offsets.clone();
        let app =
            crate::server::build_protocol_router(f.state.clone()).layer(axum::middleware::from_fn(
                move |request: axum::extract::Request, next: axum::middleware::Next| {
                    let recorded = recorded.clone();
                    async move {
                        if request.uri().path().ends_with("/share/file") {
                            let url =
                                reqwest::Url::parse(&format!("https://localhost{}", request.uri()))
                                    .unwrap();
                            for (key, value) in url.query_pairs() {
                                if key == "offset" {
                                    recorded.lock().await.push(value.parse().unwrap());
                                }
                            }
                        }
                        next.run(request).await
                    }
                },
            ));
        let task = tokio::spawn(async move {
            axum_server::bind_rustls(addr, tls)
                .handle(handle)
                .serve(app.into_make_service_with_connect_info::<std::net::SocketAddr>())
                .await
                .unwrap();
        });
        for _ in 0..100 {
            if tokio::net::TcpStream::connect(addr).await.is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let sha = hex::encode(Sha256::digest(&bytes));
        let key = hex::encode(Sha256::digest(
            serde_json::to_vec(&(&fp, &token, "nested/large.bin", &sha, bytes.len() as u64))
                .unwrap(),
        ));
        let staging = f.root.join("downloads/.antify-pulls");
        std::fs::create_dir(&staging).unwrap();
        // One full verified chunk + interrupted garbage: retry must truncate back to 4 MiB.
        let mut partial = bytes[..CHUNK as usize].to_vec();
        partial.extend_from_slice(b"incomplete");
        std::fs::write(staging.join(format!("{key}.part")), partial).unwrap();
        let q = Browse {
            target: fp.clone(),
            token: token.clone(),
            path: "nested/large.bin".into(),
            cursor: String::new(),
        };
        let result = download(&f.state, &q).await.unwrap();
        let dest = f
            .root
            .join("downloads")
            .join(result["file"].as_str().unwrap());
        assert!(dest.to_string_lossy().contains("nested"));
        assert_eq!(std::fs::read(&dest).unwrap(), bytes);
        assert!(!staging.join(format!("{key}.part")).exists());
        assert_eq!(*offsets.lock().await, vec![CHUNK, CHUNK * 2]);
        // A corrupt persisted prefix must never be published as a completed download.
        std::fs::write(staging.join(format!("{key}.part")), vec![0; CHUNK as usize]).unwrap();
        assert!(download(&f.state, &q)
            .await
            .unwrap_err()
            .to_string()
            .contains("SHA-256"));
        assert!(!staging.join(format!("{key}.part")).exists());
        // The discovered fingerprint is checked against the real TLS certificate.
        let mut forged = f.state.devices.lock().await.get(&fp).unwrap().clone();
        forged.fingerprint = "0".repeat(64);
        f.state.upsert_device(forged.clone()).await;
        assert!(remote(&f.state, &forged.fingerprint, &token).await.is_err());
        // Existing destination must never be overwritten.
        let duplicate = download(&f.state, &q).await.unwrap();
        assert_ne!(result["file"], duplicate["file"]);
        let empty = download(
            &f.state,
            &Browse {
                target: fp,
                token: token.clone(),
                path: "empty".into(),
                cursor: String::new(),
            },
        )
        .await
        .unwrap();
        assert_eq!(
            std::fs::metadata(
                f.root
                    .join("downloads")
                    .join(empty["file"].as_str().unwrap())
            )
            .unwrap()
            .len(),
            0
        );
        revoke(State(f.state.clone()), Json(Token { token })).await;
        assert!(download(&f.state, &q).await.is_err());
        shutdown.shutdown();
        task.await.unwrap();
    }
}
