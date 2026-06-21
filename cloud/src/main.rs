use std::sync::Arc;
use std::time::Duration;

use sqlx::postgres::PgPoolOptions;
use tokio::net::TcpListener;
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod config;
mod error;
mod oidc;
mod routes;
mod store;

use config::Config;
use oidc::OidcVerifier;
use routes::{build_router, AppState};
use store::{postgres::PgIdentityStore, IdentityStore};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Tracing / logging
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "seekermail_cloud=debug,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Config from environment variables
    let config = Config::from_env()?;
    tracing::info!(port = config.port, "starting seekermail-cloud");

    // Database pool
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .acquire_timeout(Duration::from_secs(5))
        .connect(&config.database_url)
        .await?;

    // Run pending migrations
    PgIdentityStore::migrate(&pool).await?;
    tracing::info!("database migrations applied");

    // Application state
    let store = Arc::new(PgIdentityStore::new(pool));
    let oidc = Arc::new(OidcVerifier::new(&config.google_oidc_audience));
    let config = Arc::new(config);

    // Spawn background task: purge expired sessions every hour.
    {
        let store_bg = Arc::clone(&store);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(3600));
            loop {
                interval.tick().await;
                match store_bg.purge_expired_sessions().await {
                    Ok(n) => tracing::info!(rows = n, "purged expired sessions"),
                    Err(e) => tracing::warn!(error = %e, "session purge failed"),
                }
            }
        });
    }

    let state = AppState {
        store,
        oidc,
        config: config.clone(),
    };

    // Build router with middleware
    let app = build_router(state)
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive()); // tighten in production

    // Start server
    let addr = format!("0.0.0.0:{}", config.port);
    let listener = TcpListener::bind(&addr).await?;
    tracing::info!(%addr, "listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

/// Wait for SIGINT or SIGTERM so Railway can do a graceful shutdown.
async fn shutdown_signal() {
    use tokio::signal;

    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    tracing::info!("shutdown signal received");
}
