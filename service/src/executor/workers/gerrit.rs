use std::{collections::HashMap, sync::Arc, time::Duration};

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use reqwest::Client;
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

const GERRIT_RESPONSE_PREFIX: &str = ")]}'";
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(1024);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Serialize, Deserialize)]
pub struct GerritConfig {
  login: String,
  password: String,
  url: String,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
struct User {
  name: String,
  email: String,
  username: String,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
struct Label {
  rejected: Option<User>,
  approved: Option<User>,
  disliked: Option<User>,
  recommended: Option<User>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
struct Labels {
  #[serde(alias = "Verified")]
  verified: Label,
  #[serde(alias = "Code-Review")]
  code_review: Label,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
struct Review {
  id: String,
  project: String,
  branch: String,
  change_id: String,
  subject: String,
  status: String,
  created: String,
  updated: String,
  submit_type: String,
  insertions: i32,
  deletions: i32,
  unresolved_comment_count: i32,
  owner: User,
  labels: Labels,
  _number: i32,
}

#[derive(Debug, Error)]
pub enum GerritError {
  #[error("Configuration error: {0}")]
  ConfigError(String),
  #[error("Authentication failed: Invalid credentials")]
  AuthError,
  #[error("Invalid API response: {0}")]
  ResponseError(String),
  #[error("Missing required field: {0}")]
  MissingFieldError(String),
}

pub struct Gerrit {
  client: Client,
  config: GerritConfig,
  action_system: Arc<ActionSystem>,
}

impl Gerrit {
  pub fn new(config: Option<Value>, action_system: Arc<ActionSystem>) -> Result<Self> {
    let config = config.ok_or_else(|| GerritError::ConfigError("Missing configuration".into()))?;

    let config =
      serde_json::from_value(config).map_err(|_| GerritError::ConfigError("Invalid configuration".into()))?;

    let client = Self::create_client()?;

    Ok(Self {
      client,
      config,
      action_system,
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

  async fn fetch_reviews(&self, project: &str, query: &str) -> Result<Vec<Review>> {
    let url = format!(
      "{}/a/changes/?q={} project:{}&o=DETAILED_ACCOUNTS&o=LABELS",
      self.config.url, query, project
    );

    let response = self
      .client
      .get(&url)
      .basic_auth(&self.config.login, Some(&self.config.password))
      .send()
      .await
      .map_err(|e| anyhow!("Failed to fetch reviews: {}", e))?;

    if !response.status().is_success() {
      return Err(anyhow!("Request failed with status: {}", response.status()));
    }

    let text = response.text().await?;
    if text == "Unauthorized" {
      return Err(GerritError::AuthError.into());
    }

    let data = text
      .strip_prefix(GERRIT_RESPONSE_PREFIX)
      .ok_or_else(|| GerritError::ResponseError("Invalid response format".into()))?;

    serde_json::from_str(data).map_err(|e| anyhow!("Failed to parse response: {}", e))
  }

  fn format_review_message(&self, review: &Review, template: &str, project: &str) -> Result<String> {
    let vars = HashMap::from([
      ("subject".to_string(), review.subject.clone()),
      ("insertions".to_string(), review.insertions.to_string()),
      ("deletions".to_string(), review.deletions.to_string()),
      ("url".to_string(), self.config.url.clone()),
      ("number".to_string(), review._number.to_string()),
      ("project".to_string(), project.to_string()),
    ]);

    strfmt(template, &vars).map_err(|e| anyhow!("Failed to format message: {}", e))
  }
}

#[derive(Debug)]
struct TaskConfig {
  project: String,
  query: String,
  template: String,
  review_template: String,
}

impl Gerrit {
  fn validate_task_config(&self, task: &Task) -> Result<TaskConfig> {
    let get_field = |field: &str| {
      task
        .options
        .get(field)
        .and_then(|v| v.as_str())
        .ok_or_else(|| GerritError::MissingFieldError(field.to_string()))
    };

    Ok(TaskConfig {
      project: get_field("project")?.to_string(),
      query: get_field("query")?.to_string(),
      template: get_field("template")?.to_string(),
      review_template: get_field("review_template")?.to_string(),
    })
  }

  fn build_message(&self, reviews: &[Review], template: &str, review_template: &str, project: &str) -> Result<String> {
    // Initialize the main message
    let vars = HashMap::from([("project".to_string(), project.to_string())]);
    let mut message = strfmt(template, &vars)?;

    // Add each review to the message
    for review in reviews {
      let review_message = self.format_review_message(review, review_template, project)?;
      message.push_str(&review_message);
    }

    Ok(message)
  }

  async fn send_notification(&self, task: &Task, message: String) -> Result<()> {
    let channel =
      get_config_value(task, "channel").ok_or_else(|| GerritError::MissingFieldError("channel".to_string()))?;

    let topic = get_config_value(task, "topic").ok_or_else(|| GerritError::MissingFieldError("topic".to_string()))?;

    self
      .action_system
      .process(Action {
        name: ZULIP.to_string(),
        options: json!({
          "message": message,
          "channel": channel,
          "topic": topic,
        })
        .to_string(),
      })
      .await
      .map_err(|e| anyhow!("Failed to send notification: {}", e))
  }
}

#[async_trait]
impl Worker for Gerrit {
  #[instrument(level = "debug", skip(self), fields(task_id = %task.id))]
  async fn execute(&self, task: &Task) -> Result<()> {
    debug!("Starting Gerrit review check");

    let config = self.validate_task_config(task)?;
    let reviews = self.fetch_reviews(&config.project, &config.query).await?;

    if !reviews.is_empty() {
      let message = self.build_message(&reviews, &config.template, &config.review_template, &config.project)?;

      self.send_notification(task, message).await?;
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
