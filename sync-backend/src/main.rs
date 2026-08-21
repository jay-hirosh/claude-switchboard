use sqlx::postgres::PgPoolOptions;

// Production builds of this binary should use `cargo build --profile
// server-release -p sync-backend` — NOT the default --release — since the
// workspace's [profile.release] sets panic="abort" for the desktop app,
// which would turn a single request-handler panic into a full process
// crash instead of a contained per-request failure.
#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let database_url = std::env::var("DATABASE_URL")
        .expect("DATABASE_URL must be set (e.g. postgres://user:pass@localhost/sync)");
    let bind_addr =
        std::env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:8787".to_string());

    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&database_url)
        .await
        .expect("connect to Postgres");

    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("run migrations");

    let app = sync_backend::app(pool);

    let listener = tokio::net::TcpListener::bind(&bind_addr)
        .await
        .expect("bind listener");
    tracing::info!("sync-backend listening on {bind_addr}");
    axum::serve(listener, app).await.expect("serve");
}
