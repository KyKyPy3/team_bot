use std::{env, sync::Arc};

use anyhow::Result;
use futures::FutureExt;
use sqlx::sqlite::SqlitePoolOptions;
use tokio::signal;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, subscriber};
use tracing_bunyan_formatter::BunyanFormattingLayer;
use tracing_log::LogTracer;
use tracing_subscriber::{layer::SubscriberExt, EnvFilter};

use team_bot_service::executor::executor::ExecutorSystem;
use team_bot_service::jobs::{clean, exchange};

mod utils;

#[tokio::main]
async fn main() -> Result<()> {
  dotenvy::dotenv().ok();

  let log_level = env::var("TEAM_BOT_LOG_LEVEL").expect("TEAM_BOT_LOG_LEVEL is not set in .env file");
  let db_url = env::var("DATABASE_URL").expect("DATABASE_URL is not set in .env file");

  let file_appender = tracing_appender::rolling::daily("logs", "app.log");
  let (file_appender, _guard) = tracing_appender::non_blocking(file_appender);
  let subscriber = tracing_subscriber::registry()
    .with(EnvFilter::from_default_env().add_directive(log_level.parse()?))
    // .with(JsonStorageLayer)
    .with(BunyanFormattingLayer::new("app".into(), std::io::stdout))
    .with(BunyanFormattingLayer::new("app".into(), file_appender));

  LogTracer::init()?;

  subscriber::set_global_default(subscriber)?;

  let cancel_token = CancellationToken::new();

  // Start task for catching interrupt
  tokio::spawn({
    let cancel_token = cancel_token.clone();
    async move {
      let ctrl_c = async {
        signal::ctrl_c().await.expect("failed to install Ctrl+C handler");
      };

      #[cfg(unix)]
      let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
          .expect("failed to install signal handler")
          .recv()
          .await;
      };

      #[cfg(not(unix))]
      let terminate = std::future::pending::<()>();

      tokio::select! {
        _ = ctrl_c => {
          info!("Received Ctrl-C, shutting down...");
          cancel_token.cancel()
        },
        _ = terminate => {
          info!("Received terminate, shutting down...");
          cancel_token.cancel()
        },
      }
    }
  });

  let pool = SqlitePoolOptions::new()
    .max_connections(100)
    .min_connections(5)
    .connect(&db_url)
    .await
    .expect("Database connection failed");

  let shared_pool = Arc::new(pool);
  let executor_system = ExecutorSystem::new(shared_pool.clone()).await?;

  if let Err(err) = utils::join_all(
    vec![
      team_bot_api::run(shared_pool.clone(), cancel_token.clone()).boxed(),
      exchange::run(shared_pool.clone(), cancel_token.clone()).boxed(),
      clean::run(shared_pool.clone(), cancel_token.clone()).boxed(),
      executor_system.run(cancel_token.clone()).boxed(),
    ],
    cancel_token,
  )
  .await
  {
    error!("One of main thread get error while execution: {:?}", err);
  }

  shared_pool.close().await;

  Ok(())
}
