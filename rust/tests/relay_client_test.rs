use antify_relay::{token_hash, App, Config, Device};
use antify_rs::{
    config::Identity,
    relay, server,
    state::{now_ms, ChatMsg, Device as LanDevice, Shared},
};
use reqwest::Client;
use serde_json::{json, Value};
use std::{path::PathBuf, time::Duration};

struct Node {
    root: PathBuf,
    state: Shared,
    panel: String,
    panel_task: tokio::task::JoinHandle<()>,
    relay_task: tokio::task::JoinHandle<()>,
}
impl Node {
    async fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("antify-client-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("downloads")).unwrap();
        Self::open(root).await
    }
    async fn open(root: PathBuf) -> Self {
        if !root.join("cert.pem").exists() {
            let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
            std::fs::write(root.join("cert.pem"), cert.cert.pem()).unwrap();
            std::fs::write(root.join("key.pem"), cert.key_pair.serialize_pem()).unwrap();
        }
        use base64::Engine;
        let pem = std::fs::read_to_string(root.join("cert.pem")).unwrap();
        let der = base64::engine::general_purpose::STANDARD
            .decode(
                pem.lines()
                    .filter(|l| !l.starts_with("---"))
                    .collect::<String>(),
            )
            .unwrap();
        let fingerprint =
            hex::encode_upper(ring::digest::digest(&ring::digest::SHA256, &der).as_ref());
        let state = server::new_state(Identity {
            alias: "test".into(),
            port: 0,
            fingerprint,
            device_model: "test".into(),
            download_dir: root.join("downloads"),
            cert_pem: root.join("cert.pem"),
            key_pem: root.join("key.pem"),
            cfg_dir: root.clone(),
        })
        .unwrap();
        state.load_persisted().await;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let panel = format!("http://{}", listener.local_addr().unwrap());
        let router = server::build_panel_router(state.clone());
        let panel_task = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let relay_task = relay::start(state.clone());
        Self {
            root,
            state,
            panel,
            panel_task,
            relay_task,
        }
    }
    async fn restart(&mut self) {
        self.relay_task.abort();
        self.panel_task.abort();
        if !self.relay_task.is_finished() {
            let _ = (&mut self.relay_task).await;
        }
        if !self.panel_task.is_finished() {
            let _ = (&mut self.panel_task).await;
        }
        let mut next = Self::open(self.root.clone()).await;
        std::mem::swap(self, &mut next);
        // Old object must not remove the new node's persistent files.
        next.root = PathBuf::new();
    }
}
impl Drop for Node {
    fn drop(&mut self) {
        self.panel_task.abort();
        self.relay_task.abort();
        if !self.root.as_os_str().is_empty() {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
}
async fn panel(client: &Client, node: &Node) -> Value {
    client
        .get(format!("{}/api/ui/state?focused=1", node.panel))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap()
}
async fn configure(client: &Client, node: &Node, url: &str, token: &str) {
    client
        .post(format!("{}/api/ui/relay", node.panel))
        .json(&json!({"enabled":true,"server_url":url,"token":token,
            "prefer_p2p":std::env::var_os("ANTIFY_FORCE_P2P_TEST").is_some()}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
}
async fn wait(mut check: impl FnMut() -> bool) {
    tokio::time::timeout(
        Duration::from_secs(if std::env::var_os("ANTIFY_P2P_LARGE_TEST").is_some() {
            900
        } else {
            40
        }),
        async {
            while !check() {
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        },
    )
    .await
    .unwrap();
}
async fn roundtrip(url: &str, atoken: &str, btoken: &str, bid: &str) {
    antify_rs::install_crypto_provider();
    let client = Client::builder()
        .timeout(Duration::from_secs(
            if std::env::var_os("ANTIFY_P2P_LARGE_TEST").is_some() {
                180
            } else {
                20
            },
        ))
        .build()
        .unwrap();
    let mut a = Node::new().await;
    let mut b = Node::new().await;
    let scope = relay::Settings {
        enabled: true,
        server_url: url.to_owned(),
        token: atoken.to_owned(),
        prefer_p2p: false,
    }
    .scope();
    let old_history = ChatMsg {
        id: 0,
        out: true,
        peer: format!("relay:{scope}:{bid}"),
        peer_alias: "test".into(),
        kind: "text".into(),
        text: "legacy history".into(),
        name: String::new(),
        size: 0,
        at: now_ms(),
        file: String::new(),
        status: "ok".into(),
        src_path: String::new(),
    };
    let legacy_history_id = a.state.db.insert_msg(&old_history).unwrap();
    let lan_history_id = a
        .state
        .db
        .insert_msg(&ChatMsg {
            peer: b.state.identity.fingerprint.clone(),
            text: "LAN history".into(),
            ..old_history
        })
        .unwrap();
    configure(&client, &a, url, atoken).await;
    configure(&client, &b, url, btoken).await;
    let peer = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let s = panel(&client, &a).await;
            if s["relay"]["connected"] == true {
                if let Some(p) = s["devices"].as_array().unwrap().iter().find(|p| {
                    p["transport"] == "p2p"
                        && p["online"] == true
                        && p["legacyPeer"]
                            .as_str()
                            .unwrap()
                            .ends_with(&format!(":{bid}"))
                }) {
                    break p["fingerprint"].as_str().unwrap().to_string();
                }
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(a.state.db.get_msg(legacy_history_id).unwrap().peer, peer);
    assert_eq!(a.state.db.get_msg(lan_history_id).unwrap().peer, peer);
    let lan = LanDevice {
        fingerprint: peer.clone(),
        alias: "LAN name".into(),
        ip: "127.0.0.1".parse().unwrap(),
        port: 53317,
        https: true,
        device_model: None,
        device_type: None,
        version: "2.1".into(),
        download: false,
        last_seen: now_ms(),
    };
    a.state.upsert_device(lan.clone()).await;
    a.state
        .upsert_device(LanDevice {
            fingerprint: "a-different-device".into(),
            ..lan.clone()
        })
        .await;
    let s = panel(&client, &a).await;
    let entries: Vec<_> = s["devices"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["alias"] == "LAN name")
        .collect();
    assert_eq!(entries.len(), 2);
    let merged: Vec<_> = entries
        .into_iter()
        .filter(|d| d["fingerprint"] == peer)
        .collect();
    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0]["p2p"], true);
    assert_ne!(merged[0]["transport"], "p2p");
    a.state
        .upsert_device(LanDevice {
            last_seen: 0,
            ..lan
        })
        .await;
    assert!(
        !s.to_string().contains(atoken),
        "state must not expose the token"
    );
    assert!(a.root.join("relay.json").exists());
    configure(&client, &a, url, "").await;
    assert_eq!(
        serde_json::from_slice::<Value>(&std::fs::read(a.root.join("relay.json")).unwrap())
            .unwrap()["token"],
        atoken
    );
    let unique = uuid::Uuid::new_v4().to_string();
    let sent: Value = client
        .post(format!("{}/api/ui/send-text", a.panel))
        .json(&json!({"target":peer,"text":unique}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let local = sent["id"].as_u64().unwrap();
    wait(|| a.state.db.get_msg(local).is_some_and(|m| m.status == "ok")).await;
    assert_eq!(
        b.state
            .db
            .recent_msgs(100)
            .iter()
            .filter(|m| m.text == unique)
            .count(),
        1
    );
    let large = std::env::var_os("ANTIFY_P2P_LARGE_TEST").is_some();
    let concurrent = large || std::env::var_os("ANTIFY_P2P_CONCURRENT_TEST").is_some();
    let size = if large {
        2 * 1024 * 1024 * 1024
    } else if concurrent {
        32 * 1024 * 1024
    } else if std::env::var_os("ANTIFY_P2P_DIAG_TEST").is_some() {
        9 * 1024 * 1024
    } else {
        1024 * 1024
    };
    let source = a.root.join("远程测试.bin");
    if large {
        std::fs::File::create(&source)
            .unwrap()
            .set_len(size)
            .unwrap();
    } else {
        std::fs::write(&source, vec![0x67u8; size as usize]).unwrap();
    }
    let sent: Value = client
        .post(format!("{}/api/ui/send-path", a.panel))
        .json(&json!({"target":peer,"path":source}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let file_local = sent["ids"][0].as_u64().unwrap();
    if concurrent {
        wait(|| {
            let progress = a.state.relay.progress.lock().unwrap();
            progress.sent >= 1024 * 1024 && progress.sent < size
        })
        .await;
        let receiving = panel(&client, &b).await;
        let active = receiving["p2pReceiving"].as_array().unwrap();
        assert!(active.iter().any(|p| p["name"] == "远程测试.bin"
            && p["file_type"] == "BIN"
            && p["total"] == size
            && p["verifying"] == false
            && p["relayed"] == std::env::var_os("ANTIFY_FORCE_P2P_TEST").is_none()
            && p["got"].as_u64().is_some_and(|got| got > 0 && got < size)));
        if std::env::var_os("ANTIFY_FORCE_P2P_TEST").is_some() {
            let during_file = uuid::Uuid::new_v4().to_string();
            let text: Value = client
                .post(format!("{}/api/ui/send-text", a.panel))
                .json(&json!({"target":peer,"text":during_file}))
                .send()
                .await
                .unwrap()
                .error_for_status()
                .unwrap()
                .json()
                .await
                .unwrap();
            let text_id = text["id"].as_u64().unwrap();
            tokio::time::timeout(
                Duration::from_secs(15),
                wait(|| {
                    a.state
                        .db
                        .get_msg(text_id)
                        .is_some_and(|m| m.status == "ok")
                }),
            )
            .await
            .expect("text should arrive while a file is in progress");
            assert!(b
                .state
                .db
                .recent_msgs(100)
                .iter()
                .any(|m| m.text == during_file));
            assert_ne!(a.state.db.get_msg(file_local).unwrap().status, "ok");
        }
        b.restart().await;
        a.restart().await;
    }
    wait(|| {
        a.state
            .db
            .get_msg(file_local)
            .is_some_and(|m| m.status == "ok")
    })
    .await;
    let file = b
        .state
        .db
        .recent_msgs(100)
        .into_iter()
        .find(|m| m.name == "远程测试.bin")
        .unwrap();
    let received = b.root.join("downloads").join(file.file);
    assert_eq!(std::fs::metadata(&received).unwrap().len(), size);
    if concurrent {
        assert!(a
            .state
            .events
            .lock()
            .await
            .iter()
            .any(|(_, s)| s.contains("点对点续传起点")));
    }
    if large {
        fn digest(path: &std::path::Path) -> Vec<u8> {
            let mut file = std::fs::File::open(path).unwrap();
            use std::io::Read;
            let mut hash = ring::digest::Context::new(&ring::digest::SHA256);
            let mut buf = vec![0; 256 * 1024];
            loop {
                let count = file.read(&mut buf).unwrap();
                if count == 0 {
                    break;
                }
                hash.update(&buf[..count]);
            }
            hash.finish().as_ref().to_vec()
        }
        assert_eq!(digest(&source), digest(&received));
    } else {
        assert_eq!(std::fs::read(received).unwrap(), vec![0x67; size as usize]);
    }

    // Browser drag/drop body is staged durably, and cleaned up after delivery.
    let staged: Value = client
        .post(format!("{}/api/ui/send", a.panel))
        .query(&[
            ("target", peer.as_str()),
            ("name", "drop.bin"),
            ("size", "3"),
        ])
        .body("123")
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let staged_id = staged["id"].as_u64().unwrap();
    let staged_path = a.state.db.get_msg(staged_id).unwrap().src_path;
    wait(|| {
        a.state
            .db
            .get_msg(staged_id)
            .is_some_and(|m| m.status == "ok")
            && !std::path::Path::new(&staged_path).exists()
    })
    .await;

    b.relay_task.abort();
    let _ = (&mut b.relay_task).await;
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let view = panel(&client, &a).await;
            if view["devices"]
                .as_array()
                .unwrap()
                .iter()
                .any(|d| d["fingerprint"] == peer && d["online"] == false)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();
    let offline = uuid::Uuid::new_v4().to_string();
    let sent: Value = client
        .post(format!("{}/api/ui/send-text", a.panel))
        .json(&json!({"target":peer,"text":offline}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let offline_id = sent["id"].as_u64().unwrap();
    wait(|| {
        a.state
            .db
            .get_msg(offline_id)
            .is_some_and(|m| m.status == "queued")
    })
    .await;
    assert!(!b
        .state
        .db
        .recent_msgs(100)
        .iter()
        .any(|m| m.text == offline));
    let changed_path = a.root.join("changed.bin");
    std::fs::write(&changed_path, b"old").unwrap();
    let changed: Value = client
        .post(format!("{}/api/ui/send-path", a.panel))
        .json(&json!({"target":peer,"path":changed_path}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let changed_id = changed["ids"][0].as_u64().unwrap();
    std::fs::write(&changed_path, b"new").unwrap();
    a.restart().await;
    b.restart().await;
    wait(|| {
        a.state
            .db
            .get_msg(offline_id)
            .is_some_and(|m| m.status == "ok")
    })
    .await;
    assert_eq!(
        b.state
            .db
            .recent_msgs(100)
            .iter()
            .filter(|m| m.text == offline)
            .count(),
        1
    );
    wait(|| {
        a.state
            .db
            .get_msg(changed_id)
            .is_some_and(|m| m.status == "fail")
    })
    .await;
    assert!(a.state.db.direct_failures()[&changed_id].contains("源文件已修改"));
    assert!(b
        .state
        .db
        .recent_msgs(100)
        .iter()
        .all(|m| m.name != "changed.bin"));
    // Restarted receiver must not reinsert messages whose ACK was already sent.
    b.restart().await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(
        b.state
            .db
            .recent_msgs(100)
            .iter()
            .filter(|m| m.text == offline)
            .count(),
        1
    );
}

#[tokio::test]
async fn panel_to_p2p_to_other_client_and_restart() {
    let root = std::env::temp_dir().join(format!("antify-client-server-{}", uuid::Uuid::new_v4()));
    let a = "a".repeat(40);
    let b = "b".repeat(40);
    let cfg = Config {
        data_dir: root.clone(),
        devices: vec![
            Device {
                id: "a".into(),
                alias: "A".into(),
                token_sha256: token_hash(&a),
            },
            Device {
                id: "b".into(),
                alias: "B".into(),
                token_sha256: token_hash(&b),
            },
        ],
        ..Default::default()
    };
    let app = App::open(cfg).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, antify_relay::router(app))
            .await
            .unwrap();
    });
    roundtrip(&url, &a, &b, "b").await;
    assert_eq!(std::fs::read_dir(root.join("files")).unwrap().count(), 0);
    let messages: Vec<Value> = Client::new()
        .get(format!("{url}/v1/messages"))
        .bearer_auth(&b)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        messages.is_empty(),
        "direct payload must not be stored on signaling server"
    );
    task.abort();
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
#[ignore = "requires the deployed server's SSH tunnel and private credentials"]
async fn deployed_client_roundtrip() {
    let url = std::env::var("ANTIFY_RELAY_TEST_URL").unwrap();
    let path = std::env::var("ANTIFY_RELAY_TEST_CREDENTIALS").unwrap();
    let v: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let a = &v["devices"][0];
    let b = &v["devices"][1];
    roundtrip(
        &url,
        a["token"].as_str().unwrap(),
        b["token"].as_str().unwrap(),
        b["id"].as_str().unwrap(),
    )
    .await;
}
