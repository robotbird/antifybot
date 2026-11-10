use antify_relay::{protocol::*, token_hash, App, Config, Device};
use futures_util::{SinkExt, StreamExt};
use reqwest::{Client, StatusCode};
use serde_json::{json, Value};
use std::{path::PathBuf, time::Duration};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, Message as Frame},
};

const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const C: &str = "cccccccccccccccccccccccccccccccccccccccccccccccc";

struct Harness {
    url: String,
    config: Config,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Harness {
    fn drop(&mut self) {
        self.task.abort();
        let _ = std::fs::remove_dir_all(&self.config.data_dir);
    }
}
async fn start(config: Config) -> (String, tokio::task::JoinHandle<()>) {
    let app = App::open(config).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, antify_relay::router(app))
            .await
            .unwrap();
    });
    (url, task)
}
async fn harness() -> Harness {
    let config = Config {
        data_dir: std::env::temp_dir().join(format!("antify-relay-test-{}", uuid::Uuid::new_v4())),
        devices: [("a", A), ("b", B), ("c", C)]
            .into_iter()
            .map(|(id, token)| Device {
                id: id.into(),
                alias: id.into(),
                token_sha256: token_hash(token),
            })
            .collect(),
        max_file_bytes: 2 * 1024 * 1024,
        max_storage_bytes: 2 * 1024 * 1024,
        ..Default::default()
    };
    let (url, task) = start(config.clone()).await;
    Harness { url, config, task }
}
async fn events(client: &Client, url: &str, token: &str) -> Vec<Value> {
    client
        .get(format!("{url}/v1/messages"))
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap()
}

// The same authenticated message/file/WebSocket checks can run against an SSH tunnel.
async fn exercise(url: &str, a: &str, b: &str, aid: &str, bid: &str) {
    let client = Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .unwrap();
    assert_eq!(
        client
            .get(format!("{url}/health"))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        client
            .get(format!("{url}/v1/devices"))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        client
            .get(format!("{url}/v1/devices"))
            .bearer_auth("invalid")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let id = uuid::Uuid::new_v4().to_string();
    let request = json!({"id":id,"to":bid,"body":{"kind":"text","text":"跨网络测试 🐜"}});
    let send = || {
        client
            .post(format!("{url}/v1/messages"))
            .bearer_auth(a)
            .json(&request)
    };
    assert_eq!(send().send().await.unwrap().status(), StatusCode::OK);
    assert_eq!(send().send().await.unwrap().status(), StatusCode::OK);
    let inbox = events(&client, url, b).await;
    assert_eq!(inbox.iter().filter(|e| e["message"]["id"] == id).count(), 1);
    let mut changed = request.clone();
    changed["body"]["text"] = json!("changed");
    assert_eq!(
        client
            .post(format!("{url}/v1/messages"))
            .bearer_auth(a)
            .json(&changed)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        client
            .post(format!("{url}/v1/messages/{id}/ack"))
            .bearer_auth(a)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        client
            .post(format!("{url}/v1/messages/{id}/ack"))
            .bearer_auth(b)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    assert!(events(&client, url, a)
        .await
        .iter()
        .any(|e| e["type"] == "delivered" && e["id"] == id));
    assert_eq!(
        client
            .post(format!("{url}/v1/messages/{id}/ack"))
            .bearer_auth(a)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );

    // More than the JSON body limit, proving files are streamed separately.
    let bytes = vec![0xa7u8; 1024 * 1024];
    let hash = token_hash_bytes(&bytes);
    let file: FileInfo = client
        .post(format!("{url}/v1/files"))
        .bearer_auth(a)
        .query(&[("to", bid), ("name", "中文测试.bin")])
        .header("x-file-sha256", &hash)
        .body(bytes.clone())
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(file.sha256, hash);
    let downloaded = client
        .get(format!("{url}/v1/files/{}", file.id))
        .bearer_auth(b)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    assert_eq!(downloaded.headers()["x-file-sha256"], hash);
    assert_eq!(downloaded.bytes().await.unwrap().as_ref(), bytes);
    let file_id = uuid::Uuid::new_v4().to_string();
    let file_message = json!({"id":file_id,"to":bid,"body":{"kind":"file","file_id":file.id}});
    assert_eq!(
        client
            .post(format!("{url}/v1/messages"))
            .bearer_auth(a)
            .json(&file_message)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        client
            .post(format!("{url}/v1/messages/{file_id}/ack"))
            .bearer_auth(b)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        client
            .post(format!("{url}/v1/messages/{file_id}/ack"))
            .bearer_auth(a)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );

    let wsurl = format!("{}/v1/ws", url.replacen("http", "ws", 1));
    let mut req = wsurl.as_str().into_client_request().unwrap();
    req.headers_mut()
        .insert("authorization", format!("Bearer {b}").parse().unwrap());
    let (mut socket, _) = connect_async(req).await.unwrap();
    let ready = socket.next().await.unwrap().unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(ready.to_text().unwrap()).unwrap()["device_id"],
        bid
    );
    let wsid = uuid::Uuid::new_v4().to_string();
    socket
        .send(Frame::Text(
            serde_json::to_string(&ClientEvent::Send {
                message: SendMessage {
                    id: wsid.clone(),
                    to: aid.into(),
                    body: Body::Text {
                        text: "WebSocket reply".into(),
                    },
                },
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match socket.next().await.unwrap().unwrap() {
                Frame::Text(text) => {
                    let event: Value = serde_json::from_str(&text).unwrap();
                    if event["type"] == "accepted" && event["id"] == wsid {
                        break;
                    }
                }
                Frame::Ping(data) => socket.send(Frame::Pong(data)).await.unwrap(),
                _ => (),
            }
        }
    })
    .await
    .unwrap();
    assert!(events(&client, url, a)
        .await
        .iter()
        .any(|e| e["message"]["id"] == wsid));
    client
        .post(format!("{url}/v1/messages/{wsid}/ack"))
        .bearer_auth(a)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match socket.next().await.unwrap().unwrap() {
                Frame::Text(text) => {
                    let event: Value = serde_json::from_str(&text).unwrap();
                    if event["type"] == "delivered" && event["id"] == wsid {
                        break;
                    }
                }
                Frame::Ping(data) => socket.send(Frame::Pong(data)).await.unwrap(),
                _ => (),
            }
        }
    })
    .await
    .unwrap();
    socket
        .send(Frame::Text(
            serde_json::to_string(&ClientEvent::Ack { id: wsid })
                .unwrap()
                .into(),
        ))
        .await
        .unwrap();
    socket.close(None).await.unwrap();
}
fn token_hash_bytes(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}

#[tokio::test]
async fn authenticated_messages_files_and_websocket() {
    let h = harness().await;
    exercise(&h.url, A, B, "a", "b").await;
}

#[tokio::test]
async fn restart_preserves_unacknowledged_messages() {
    let mut h = harness().await;
    let client = Client::new();
    let id = uuid::Uuid::new_v4().to_string();
    client
        .post(format!("{}/v1/messages", h.url))
        .bearer_auth(A)
        .json(&json!({"id":id,"to":"b","body":{"kind":"text","text":"survives restart"}}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    h.task.abort();
    let _ = (&mut h.task).await;
    let (url, task) = start(h.config.clone()).await;
    h.url = url;
    h.task = task;
    assert!(events(&client, &h.url, B)
        .await
        .iter()
        .any(|e| e["message"]["id"] == id));
}

#[tokio::test]
async fn file_permissions_checksums_quotas_and_cleanup() {
    let h = harness().await;
    let client = Client::new();
    let response = client
        .post(format!("{}/v1/files", h.url))
        .bearer_auth(A)
        .query(&[("to", "b"), ("name", "test.bin")])
        .body(vec![42u8; 1024 * 1024])
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let file: FileInfo = response.json().await.unwrap();
    assert_eq!(
        client
            .get(format!("{}/v1/files/{}", h.url, file.id))
            .bearer_auth(C)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        client
            .post(format!(
                "{}/v1/messages/{}/ack",
                h.url,
                uuid::Uuid::new_v4()
            ))
            .bearer_auth(C)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(client.post(format!("{}/v1/messages",h.url)).bearer_auth(C).json(&json!({"id":uuid::Uuid::new_v4().to_string(),"to":"b","body":{"kind":"file","file_id":file.id}}))
        .send().await.unwrap().status(),StatusCode::BAD_REQUEST);
    let upload = || {
        client
            .post(format!("{}/v1/files", h.url))
            .bearer_auth(A)
            .query(&[("to", "b"), ("name", "test.bin")])
    };
    assert_eq!(
        upload()
            .body(vec![0; 2 * 1024 * 1024 + 1])
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::PAYLOAD_TOO_LARGE
    );
    assert_eq!(
        upload()
            .body(vec![0; 2 * 1024 * 1024])
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::INSUFFICIENT_STORAGE
    );
    assert_eq!(
        upload()
            .header("x-file-sha256", "0".repeat(64))
            .body("bad hash")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let conn = rusqlite::Connection::open(h.config.data_dir.join("relay.db")).unwrap();
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1);
    conn.execute("UPDATE files SET expires_at=0", []).unwrap();
    assert_eq!(
        client
            .get(format!("{}/v1/files/{}", h.url, file.id))
            .bearer_auth(B)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    let app = App::open(h.config.clone()).await.unwrap();
    app.cleanup(false).await.unwrap();
    assert!(!h.config.data_dir.join("files").join(file.id).exists());
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
#[ignore = "requires an explicitly configured deployed instance"]
async fn deployed_smoke() {
    let url = std::env::var("ANTIFY_RELAY_TEST_URL").expect("test URL");
    let path = PathBuf::from(
        std::env::var("ANTIFY_RELAY_TEST_CREDENTIALS").expect("credentials file path"),
    );
    let credentials: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let a = &credentials["devices"][0];
    let b = &credentials["devices"][1];
    exercise(
        &url,
        a["token"].as_str().unwrap(),
        b["token"].as_str().unwrap(),
        a["id"].as_str().unwrap(),
        b["id"].as_str().unwrap(),
    )
    .await;
}
