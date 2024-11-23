use std::{collections::HashMap, sync::Arc, time::Duration};

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use mini_moka::sync::Cache;
use reqwest::{header::HeaderMap, Client};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use strfmt::strfmt;
use thiserror::Error;
use tracing::{debug, error, instrument, warn};

use crate::executor::{
  actions::{zulip::ZULIP, Action, ActionSystem},
  workers::Worker,
};
use team_bot_entity::task::Task;

const CONNECTION_TIMEOUT: Duration = Duration::from_secs(1024);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const BUILD_CACHE_TTL: Duration = Duration::from_secs(60 * 60 * 24 * 5);
const CACHE_MAX_CAPACITY: u64 = 10_000;
const ACCEPT_HEADER: &str = "application/json";

#[derive(Debug, Error)]
pub enum TeamCityError {
  #[error("Configuration error: {0}")]
  Config(String),
  #[error("API request failed: {0}")]
  ApiRequest(String),
  #[error("Missing required field: {0}")]
  MissingFieldError(String),
}

#[derive(Debug, Serialize, Deserialize)]
struct Options {
  url: String,
}

#[derive(Deserialize, Serialize, Debug)]
struct TestOccurrences {
  passed: Option<i32>,
  failed: Option<i32>,
}

#[derive(Deserialize, Serialize, Debug)]
#[serde(rename_all = "camelCase")]
struct BuildType {
  name: String,
  project_name: String,
}

#[derive(Deserialize, Serialize, Debug)]
#[serde(rename_all = "camelCase")]
struct BuildStatusResponse {
  id: u64,
  state: String,
  status: String,
  build_type: BuildType,
  status_text: String,
  web_url: String,
  finish_date: String,
  test_occurrences: Option<TestOccurrences>,
}

/// Holds the configuration for a notification
#[derive(Debug)]
struct NotificationConfig {
  channel: Value,
  topic: Option<Value>,
}

pub struct TeamCity {
  client: Client,
  config: Options,
  action_system: Arc<ActionSystem>,
  build_cache: Cache<u64, bool>,
}

impl TeamCity {
  pub fn new(config: Option<Value>, action_system: Arc<ActionSystem>) -> Result<Self> {
    let config = config.ok_or_else(|| TeamCityError::Config("Missing configuration".into()))?;

    let config = serde_json::from_value(config).map_err(|_| TeamCityError::Config("Invalid configuration".into()))?;

    let client = Self::create_client()?;
    let build_cache = Self::create_cache();

    Ok(Self {
      client,
      config,
      action_system,
      build_cache,
    })
  }

  fn create_client() -> Result<Client> {
    Client::builder()
      .connect_timeout(CONNECTION_TIMEOUT)
      .timeout(REQUEST_TIMEOUT)
      .user_agent("Platform-Bot")
      .build()
      .map_err(|e| anyhow!(e))
  }

  fn create_cache() -> Cache<u64, bool> {
    Cache::builder()
      .time_to_idle(BUILD_CACHE_TTL)
      .max_capacity(CACHE_MAX_CAPACITY)
      .build()
  }

  async fn fetch_build_status(&self, build_name: &str) -> Result<BuildStatusResponse> {
    let mut headers = HeaderMap::new();
    headers.append("Accept", ACCEPT_HEADER.parse()?);

    let url = format!("{}/builds/buildType:{}", self.config.url, build_name);

    let response = self
      .client
      .get(url)
      .headers(headers)
      .send()
      .await
      .map_err(|e| TeamCityError::ApiRequest(e.to_string()))?;

    if !response.status().is_success() {
      return Err(TeamCityError::ApiRequest(format!("Status code: {}", response.status())).into());
    }

    response
      .json::<BuildStatusResponse>()
      .await
      .map_err(|e| TeamCityError::ApiRequest(e.to_string()).into())
  }

  async fn fetch_build_status_with_retry(&self, build_name: &str) -> Result<BuildStatusResponse> {
    let mut attempts = 0;
    while attempts < 3 {
      match self.fetch_build_status(build_name).await {
        Ok(response) => return Ok(response),
        Err(_) if attempts < 2 => {
          attempts += 1;
          tokio::time::sleep(Duration::from_secs(1 << attempts)).await;
        },
        Err(e) => return Err(e),
      }
    }
    Err(anyhow!("Max retries exceeded"))
  }

  /// Validates that all required fields are present in the task configuration
  fn validate_task_config(&self, task: &Task) -> Result<NotificationConfig> {
    let channel =
      get_config_value(task, "channel").ok_or_else(|| TeamCityError::MissingFieldError("channel".to_string()))?;

    let topic = get_config_value(task, "topic");

    Ok(NotificationConfig { channel, topic })
  }

  /// Creates a notification action from the config
  fn create_notification_action(&self, message: String, config: NotificationConfig) -> Action {
    Action {
      name: ZULIP.to_string(),
      options: json!({
        "message": message,
        "channel": config.channel,
        "topic": config.topic,
      })
      .to_string(),
    }
  }

  async fn send_notification(&self, build_info: &BuildStatusResponse, template: &str, task: &Task) -> Result<()> {
    let vars = HashMap::from([
      ("name".to_string(), build_info.build_type.name.clone()),
      ("project_name".to_string(), build_info.build_type.project_name.clone()),
      ("status".to_string(), build_info.status_text.clone()),
      ("web_url".to_string(), build_info.web_url.clone()),
    ]);

    let message = strfmt(template, &vars)?;
    let config = self.validate_task_config(task).map_err(|e| {
      error!("Configuration error: {}", e);
      anyhow!(e)
    })?;

    let action = self.create_notification_action(message, config);

    self.action_system.process(action).await.map_err(|e| anyhow!(e))
  }

  fn should_notify(&self, build: &BuildStatusResponse) -> bool {
    build.state == "finished" && build.status == "FAILURE" && !self.build_cache.contains_key(&build.id)
  }
}

#[async_trait]
impl Worker for TeamCity {
  #[instrument(level = "debug", skip(self), fields(task_id = %task.id, task_name = %task.name))]
  async fn execute(&self, task: &Task) -> Result<()> {
    debug!("Executing TeamCity for {}", task.id);

    let build_name = task
      .options
      .get("build")
      .and_then(|v| v.as_str())
      .ok_or_else(|| TeamCityError::MissingFieldError("build".into()))?;

    let build_status = self.fetch_build_status(build_name).await?;

    if self.should_notify(&build_status) {
      self.build_cache.insert(build_status.id, true);

      let template = task
        .options
        .get("template")
        .and_then(|v| v.as_str())
        .ok_or_else(|| TeamCityError::MissingFieldError("template".into()))?;

      self.send_notification(&build_status, template, task).await?;
    }

    Ok(())
  }
}

/// Retrieves a configuration value from either task or project settings
#[instrument(level = "debug", skip(task))]
fn get_config_value(task: &Task, key: &str) -> Option<Value> {
  task
    .options
    .get(key)
    .cloned()
    .or_else(|| task.project.options.get(task.r#type.as_str())?.get(key).cloned())
}
