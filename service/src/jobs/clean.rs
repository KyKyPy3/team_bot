use std::sync::Arc;

use anyhow::Result;
use sqlx::SqlitePool;
use tokio::select;
use tokio::time::{sleep, Duration};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info};

static QUERY_TIMEOUT: Duration = Duration::from_secs(15);

pub async fn run(pool: Arc<SqlitePool>, cancel_token: CancellationToken) -> Result<()> {
  debug!("Cleaning database jobs started");

  while !cancel_token.is_cancelled() {
    select! {
      biased;
      _ = cancel_token.cancelled() => {
        info!("Cleaning database jobs stopped");
        break;
      }
      _ = sleep(QUERY_TIMEOUT) => {
        let affected_tasks = crate::mutation::tasks::delete_completed_tasks(&pool).await;

        if let Err(e) = affected_tasks {
          error!("Failed to delete tasks: {}", e);

          continue;
        }

        debug!("Delete {} completed tasks", affected_tasks?);
      }
    }
  }

  Ok(())
}
