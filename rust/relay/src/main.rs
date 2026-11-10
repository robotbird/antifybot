use antify_relay::{App, Config};
use std::time::Duration;

fn main() -> anyhow::Result<()> {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(4)
        .enable_all()
        .build()?
        .block_on(run())
}

async fn run() -> anyhow::Result<()> {
    let config_path = std::env::var("ANTIFY_RELAY_CONFIG").unwrap_or_else(|_| "server.json".into());
    let config: Config = serde_json::from_slice(&tokio::fs::read(config_path).await?)?;
    let listener = tokio::net::TcpListener::bind(&config.listen).await?;
    let app = App::open(config).await?;
    let cleanup_app = app.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        loop {
            interval.tick().await;
            if let Err(e) = cleanup_app.cleanup(false).await {
                eprintln!("cleanup failed: {}", e.1);
            }
        }
    });
    eprintln!(
        "antify-relay {} listening on {}",
        env!("CARGO_PKG_VERSION"),
        listener.local_addr()?
    );
    axum::serve(listener, antify_relay::router(app))
        .with_graceful_shutdown(shutdown())
        .await?;
    Ok(())
}

async fn shutdown() {
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("SIGTERM handler");
        tokio::select! { _ = tokio::signal::ctrl_c() => (), _ = term.recv() => () }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
