use std::{collections::HashMap, str::FromStr, sync::Arc, time::Duration};

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use cron::Schedule;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tokio::{
  sync::{
    mpsc::{channel, Receiver, Sender},
    Mutex,
  },
  time::sleep,
};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, instrument, warn};

use crate::{
  executor::{
    actions::{ActionConfig, ActionSystem},
    workers::{gerrit::Gerrit, notifier::Notifier, teamcity::TeamCity, Worker, WorkerConfig},
  },
  mutation, query,
};
use team_bot_entity::task::Task;

const QUERY_TIMEOUT: Duration = Duration::from_secs(5);
const CHANNEL_CAPACITY: usize = 500;

#[derive(Debug, Serialize, Deserialize)]
struct Config {
  num_workers: u32,
  workers: Vec<WorkerConfig>,
  actions: Vec<ActionConfig>,
}

impl Config {
  fn from_file(path: &str) -> Result<Self> {
    let file = std::fs::File::open(path).with_context(|| format!("Failed to open config file: {}", path))?;

    serde_json::from_reader(file).with_context(|| "Failed to parse config file")
  }
}

pub struct ExecutorSystem {
  config: Config,
  pool: Arc<SqlitePool>,
  workers: Arc<HashMap<String, Box<dyn Worker>>>,
  action_system: Arc<ActionSystem>,
  tx: Sender<Task>,
  rx: Arc<Mutex<Receiver<Task>>>,
}

impl ExecutorSystem {
  #[instrument(level = "debug", skip(pool))]
  pub async fn new(pool: Arc<SqlitePool>) -> Result<Self> {
    let (tx, rx) = channel::<Task>(CHANNEL_CAPACITY);

    let config = Config::from_file("config.json")?;
    let action_system = Arc::new(ActionSystem::new(config.actions.clone()).await?);
    let workers = Self::initialize_workers(&config.workers, action_system.clone())?;

    Ok(Self {
      config,
      pool,
      action_system,
      workers: Arc::new(workers),
      tx,
      rx: Arc::new(Mutex::new(rx)),
    })
  }

  fn initialize_workers(
    configs: &[WorkerConfig],
    action_system: Arc<ActionSystem>,
  ) -> Result<HashMap<String, Box<dyn Worker>>> {
    let mut workers = HashMap::new();

    for config in configs {
      let worker: Box<dyn Worker> = match config.name.as_str() {
        "notify" => match Notifier::new(config.options.clone(), action_system.clone()) {
          Ok(notifier) => Box::new(notifier),
          Err(err) => {
            return Err(anyhow!("Can't create {} worker. Err: {}", config.name.as_str(), err));
          },
        },
        "teamcity" => match TeamCity::new(config.options.clone(), action_system.clone()) {
          Ok(teamcity) => Box::new(teamcity),
          Err(err) => {
            return Err(anyhow!("Can't create {} worker. Err: {}", config.name.as_str(), err));
          },
        },
        "gerrit" => match Gerrit::new(config.options.clone(), action_system.clone()) {
          Ok(gerrit) => Box::new(gerrit),
          Err(err) => {
            return Err(anyhow!("Can't create {} worker. Err: {}", config.name.as_str(), err));
          },
        },
        unknown => {
          warn!("Unknown worker type: {}", unknown);
          continue;
        },
      };

      workers.insert(config.name.clone(), worker);
    }

    Ok(workers)
  }

  #[instrument(level = "debug", skip(self, cancel_token))]
  pub async fn run(self, cancel_token: CancellationToken) -> Result<()> {
    let mut handlers = vec![];
    info!("Starting executor...");

    self.action_system.run(cancel_token.clone()).await?;
    handlers.push(self.spawn_task_poller(cancel_token.clone()));
    handlers.extend(self.spawn_workers(cancel_token));

    info!("Executor started");

    futures::future::join_all(handlers).await;
    info!("Executor system stopped");

    Ok(())
  }

  fn spawn_task_poller(&self, cancel_token: CancellationToken) -> tokio::task::JoinHandle<()> {
    let pool = self.pool.clone();
    let tx = self.tx.clone();

    tokio::spawn(async move {
      info!("Task poller started");

      while !cancel_token.is_cancelled() {
        tokio::select! {
          _ = sleep(QUERY_TIMEOUT) => {
            debug!("Start polling task from db...");

            match query::tasks::get_tasks_to_run(&pool).await {
              Ok(tasks) => {
                debug!("Found {} tasks to run", tasks.len());
                for task in tasks {
                  if let Err(e) = tx.send(task).await {
                    error!("Failed to send task to executor: {}", e);
                  }
                }
              },
              Err(e) => error!("Failed to get tasks to run: {}", e),
            }
          }
          _ = cancel_token.cancelled() => {
            info!("Task poller stopped");
            break;
          }
        }
      }
    })
  }

  fn spawn_workers(&self, cancel_token: CancellationToken) -> Vec<tokio::task::JoinHandle<()>> {
    info!("Starting {} workers...", self.config.num_workers);

    let handlers = (0..self.config.num_workers)
      .map(|id| self.spawn_worker(id, cancel_token.clone()))
      .collect();

    info!("Workers started");

    handlers
  }

  #[instrument(level = "debug", skip(self, cancel_token))]
  fn spawn_worker(&self, id: u32, cancel_token: CancellationToken) -> tokio::task::JoinHandle<()> {
    let rx = Arc::clone(&self.rx);
    let workers = self.workers.clone();
    let pool = self.pool.clone();

    tokio::spawn(async move {
      loop {
        let mut rx = rx.lock().await;

        tokio::select! {
          Some(task) = rx.recv() => {
            debug!("Worker {} received task {:?}", id, task);

            if let Err(e) = Self::process_task(&pool, &workers, task).await {
              error!("Worker {} failed to process task: {}", id, e);
            }
          }
          _ = cancel_token.cancelled() => {
            info!("Worker {} stopped", id);
            break;
          }
        }
      }
    })
  }

  #[instrument(level = "debug", skip(pool, workers))]
  async fn process_task(pool: &SqlitePool, workers: &HashMap<String, Box<dyn Worker>>, task: Task) -> Result<()> {
    mutation::tasks::run_task(pool, task.id)
      .await
      .context("Failed to set task status to in_progress")?;

    let worker = workers
      .get(&task.r#type)
      .ok_or_else(|| anyhow::anyhow!("Unknown worker type: {}", task.r#type))?;

    match worker.execute(&task).await {
      Ok(_) => {
        if let Some(_) = &task.schedule {
          let start_at = calculate_next_run(&task).context("Failed to calculate next run time")?;

          mutation::tasks::schedule_task(pool, task.id, start_at)
            .await
            .context("Failed to schedule next task run")?;
        } else {
          mutation::tasks::completed_task(pool, task.id)
            .await
            .context("Failed to mark task as completed")?;
        }
      },
      Err(e) => {
        error!("Task execution failed: {}", e);
        mutation::tasks::failed_task(pool, task.id)
          .await
          .context("Failed to mark task as failed")?;
      },
    }

    Ok(())
  }
}

#[instrument(level = "debug")]
fn calculate_next_run(task: &Task) -> Result<i32> {
  let start_at =
    DateTime::from_timestamp(task.start_at as i64, 0).ok_or_else(|| anyhow::anyhow!("Invalid timestamp"))?;

  let next_run = if let Some(schedule) = &task.schedule {
    if schedule.starts_with("@every") {
      calculate_interval_next_run(schedule, start_at)?
    } else {
      calculate_cron_next_run(schedule, start_at)?
    }
  } else {
    start_at.timestamp() as i32
  };

  Ok(next_run.max(Utc::now().timestamp() as i32))
}

fn calculate_interval_next_run(schedule: &str, start_at: DateTime<Utc>) -> Result<i32> {
  let duration_str = schedule.trim_start_matches("@every ");
  let duration = duration_str::parse(duration_str).map_err(|e| anyhow!("Failed to parse duration: {}", e))?;

  let duration = chrono::Duration::from_std(duration).context("Failed to convert duration")?;

  Ok((start_at.timestamp() + duration.num_seconds()) as i32)
}

fn calculate_cron_next_run(schedule: &str, start_at: DateTime<Utc>) -> Result<i32> {
  let schedule = Schedule::from_str(schedule).context("Failed to parse cron schedule")?;

  let next_run = schedule
    .after(&start_at)
    .next()
    .ok_or_else(|| anyhow::anyhow!("Failed to calculate next cron run"))?;

  Ok(next_run.timestamp() as i32)
}
