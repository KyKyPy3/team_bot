use std::sync::Arc;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use thiserror::Error;
use tracing::{debug, error, instrument};
use uuid::Uuid;

use super::Worker;
use crate::executor::actions::{zulip::ZULIP, Action, ActionSystem};
use team_bot_entity::task::Task;

#[derive(Debug, Error)]
pub enum NotifierError {
  #[error("Notifier configuration error: {0}")]
  ConfigError(String),
  #[error("Missing required field: {0}")]
  MissingFieldError(String),
}

/// Holds the configuration for a notification
#[derive(Debug)]
struct NotificationConfig {
  text: Value,
  channel: Value,
  topic: Value,
}

pub struct Notifier {
  action_system: Arc<ActionSystem>,
}

impl Notifier {
  /// Creates a new Notifier instance
  ///
  /// # Arguments
  /// * `_config` - Optional configuration value (currently unused)
  /// * `action_system` - Reference to the action system for processing notifications
  pub fn new(_: Option<Value>, action_system: Arc<ActionSystem>) -> Result<Self> {
    Ok(Notifier { action_system })
  }

  /// Validates that all required fields are present in the task configuration
  fn validate_task_config(&self, task: &Task) -> Result<NotificationConfig> {
    let text = get_config_value(task, "text").ok_or_else(|| NotifierError::MissingFieldError("text".to_string()))?;

    let channel =
      get_config_value(task, "channel").ok_or_else(|| NotifierError::MissingFieldError("channel".to_string()))?;

    let topic = get_config_value(task, "topic").ok_or_else(|| NotifierError::MissingFieldError("topic".to_string()))?;

    Ok(NotificationConfig { text, channel, topic })
  }

  /// Creates a notification action from the config
  fn create_notification_action(&self, task_id: Uuid, config: NotificationConfig) -> Action {
    Action {
      name: ZULIP.to_string(),
      task_id,
      options: json!({
        "message": config.text,
        "channel": config.channel,
        "topic": config.topic,
      })
      .to_string(),
    }
  }
}

#[async_trait]
impl Worker for Notifier {
  #[instrument(level = "debug", skip(self), fields(task_id = %task.id, task_name = %task.name))]
  async fn execute(&self, task: &Task) -> Result<()> {
    debug!("Executing Notifier for {}", task.id);

    let config = self.validate_task_config(task).map_err(|e| {
      error!("Configuration error: {}", e);
      anyhow!(e)
    })?;

    let action = self.create_notification_action(task.id, config);

    self.action_system.process(action).await.map_err(|e| {
      error!("Failed to process notification: {}", e);
      anyhow!(e)
    })
  }
}

/// Retrieves a configuration value from either the task or project settings
///
/// # Arguments
/// * `task` - The task containing configuration
/// * `key` - The configuration key to look up
///
/// # Returns
/// * `Some(Value)` if the key exists in either task or project config
/// * `None` if the key doesn't exist in either location
#[instrument(level = "debug", skip(task))]
fn get_config_value(task: &Task, key: &str) -> Option<Value> {
  // First try task-specific options
  if let Some(value) = task.options.get(key) {
    return Some(value.clone());
  }

  // Fall back to project options
  task
    .project
    .options
    .get(&task.r#type)
    .and_then(|options| options.get(key))
    .cloned()
}

#[cfg(test)]
mod tests {
  use super::*;
  use chrono::Utc;
  use serde_json::json;
  use uuid::Uuid;

  use team_bot_entity::project::ProjectRow;

  #[test]
  fn test_get_config_value() {
    let task = Task {
      id: Uuid::new_v4(),
      status: "new".to_owned(),
      retries: 0,
      name: "task".to_owned(),
      options: json!({
        "text": "task specific text"
      }),
      external_id: None,
      external_modified_at: None,
      schedule: None,
      start_at: 0,
      created_at: Utc::now(),
      updated_at: Utc::now(),
      project: ProjectRow {
        id: Uuid::new_v4(),
        name: "name".to_owned(),
        code: "code".to_owned(),
        sync_with_exchange: 0,
        owner_id: Uuid::new_v4(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
        options: json!({
          "notification": {
            "channel": "project channel"
          }
        }),
      },
      r#type: "notification".to_string(),
    };

    assert_eq!(get_config_value(&task, "text"), Some(json!("task specific text")));
    assert_eq!(get_config_value(&task, "channel"), Some(json!("project channel")));
    assert_eq!(get_config_value(&task, "nonexistent"), None);
  }
}
