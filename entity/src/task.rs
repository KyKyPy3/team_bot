use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::prelude::FromRow;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::project::ProjectRow;

#[derive(Debug, Clone, Copy)]
pub enum TaskStatus {
  New,
  InProgress,
  Finished,
  Failed,
  Retried,
}

impl fmt::Display for TaskStatus {
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
    match self {
      TaskStatus::New => write!(f, "new"),
      TaskStatus::InProgress => write!(f, "in_progress"),
      TaskStatus::Retried => write!(f, "retried"),
      TaskStatus::Failed => write!(f, "failed"),
      TaskStatus::Finished => write!(f, "finished"),
    }
  }
}

#[derive(Serialize, Deserialize, FromRow, Debug, Clone)]
pub struct TaskRow {
  pub id: Uuid,
  pub r#type: String,
  pub status: String,
  pub project_id: Uuid,
  pub retries: i32,
  pub name: String,
  pub external_id: Option<String>,
  pub schedule: Option<String>,
  pub start_at: i32,
  pub options: Value,
  pub created_at: DateTime<Utc>,
  pub updated_at: DateTime<Utc>,
}

#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct Task {
  pub id: Uuid,
  pub r#type: String,
  pub status: String,
  pub project: ProjectRow,
  pub retries: i32,
  pub name: String,
  pub external_id: Option<String>,
  pub schedule: Option<String>,
  pub start_at: i32,
  pub options: Value,
  pub created_at: DateTime<Utc>,
  pub updated_at: DateTime<Utc>,
}
