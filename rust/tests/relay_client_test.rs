use antify_relay::{token_hash, App, Config, Device};
use antify_rs::{config::Identity, relay, server, state::Shared};
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
        let state = server::new_state(Identity {
            alias: "test".into(),
            port: 0,
            fingerprint: "TEST".into(),
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
        .json(&json!({"enabled":true,"server_url":url,"token":token}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
}
async fn wait(mut check: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(40), async {
        while !check() {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap();
}
async fn roundtrip(url: &str, atoken: &str, btoken: &str, bid: &str) {
    antify_rs::install_crypto_provider();
    let client = Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .unwrap();
    let mut a = Node::new().await;
    let mut b = Node::new().await;
    configure(&client, &a, url, atoken).await;
    configure(&client, &b, url, btoken).await;
    let peer = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let s = panel(&client, &a).await;
            if s["relay"]["connected"] == true {
                if let Some(p) = s["devices"].as_array().unwrap().iter().find(|p| {
                    p["transport"] == "relay"
                        && p["fingerprint"]
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
    let s = panel(&client, &a).await;
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
    let bytes = vec![0x67u8; 1024 * 1024];
    let source = a.root.join("远程测试.bin");
    std::fs::write(&source, &bytes).unwrap();
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
    assert_eq!(
        std::fs::read(b.root.join("downloads").join(file.file)).unwrap(),
        bytes
    );

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
            .is_some_and(|m| m.status == "accepted")
    })
    .await;
    assert!(!b
        .state
        .db
        .recent_msgs(100)
        .iter()
        .any(|m| m.text == offline));
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
async fn panel_to_relay_to_other_client_and_restart() {
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
