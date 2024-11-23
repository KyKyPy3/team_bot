use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use team_bot_entity::task::Task;

pub mod gerrit;
pub mod notifier;
pub mod teamcity;

#[derive(Debug, Serialize, Deserialize)]
pub struct WorkerConfig {
  pub name: String,
  pub options: Option<Value>,
}

#[async_trait]
pub trait Worker: Send + Sync {
  async fn execute(&self, task: &Task) -> Result<()>;
}
