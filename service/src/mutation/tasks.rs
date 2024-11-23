use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::{error::ServiceError, ServiceResult};
use team_bot_entity::{
  project::ProjectRow,
  task::{Task, TaskRow, TaskStatus},
};

// SQL Query Constants
const INSERT_TASK: &str = r#"
  INSERT INTO tasks (id, type, project_id, name, external_id, external_modified_at, schedule, start_at, options)
  VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
  ON CONFLICT (external_id) DO UPDATE SET
    name = excluded.name,
    start_at = excluded.start_at,
    schedule = excluded.schedule,
    external_modified_at = excluded.external_modified_at,
    options = excluded.options,
    updated_at = CURRENT_TIMESTAMP
  RETURNING *
"#;

const UPDATE_TASK: &str = r#"
  UPDATE tasks
  SET name = ?1, schedule = ?2, start_at = ?3, options = ?4
  WHERE id = ?5
  RETURNING *
"#;

const FIND_TASK: &str = "SELECT * FROM tasks WHERE id = ?1";
const FIND_TASK_BY_EXTARNAL_ID: &str = "SELECT * FROM tasks WHERE external_id = ?1";
const FIND_PROJECT: &str = "SELECT * FROM projects WHERE id = ?1";
const DELETE_TASK: &str = "DELETE FROM tasks WHERE id = ?";
const SCHEDULE_TASK: &str = "UPDATE tasks SET status = ?1, start_at = ?2 WHERE id = ?3 RETURNING *";
const UPDATE_TASK_STATUS: &str = "UPDATE tasks SET status = ?1 WHERE id = ?2 RETURNING *";
const DELETE_OLD_TASKS: &str = "DELETE FROM tasks WHERE status = 'finished' AND updated_at < date('now','-1 day')";
const DELETE_STALE_TASKS: &str =
  "DELETE FROM tasks WHERE external_id IS NOT NULL AND updated_at <= date('now','-10 seconds')";

#[derive(Debug, Deserialize)]
pub struct CreateTaskParams {
  pub r#type: String,
  pub name: String,
  pub project_id: Uuid,
  pub schedule: Option<String>,
  pub external_id: Option<String>,
  pub external_modified_at: Option<DateTime<Utc>>,
  pub start_at: i32,
  pub options: Value,
}

pub async fn create(pool: &SqlitePool, params: CreateTaskParams) -> ServiceResult<Task> {
  let existing_task = match &params.external_id {
    Some(external_id) => get_task_by_external_id(pool, &external_id).await?,
    None => None,
  };

  let task = create_task_row(pool, &params).await?;
  let project = get_project(pool, params.project_id).await?;

  if let Some(existing_task) = existing_task {
    let should_update = match (existing_task.external_modified_at, params.external_modified_at) {
      (Some(existing_modified_at), Some(task_modified_at)) => {
        is_status_update_needed(&existing_task, existing_modified_at, task_modified_at)
      },
      _ => false,
    };

    if should_update {
      update_task_status(pool, existing_task.id, TaskStatus::New).await?;
    }
  }

  Ok(build_task(task, project))
}

#[derive(Debug, Deserialize)]
pub struct UpdateTaskParams {
  pub name: String,
  pub schedule: Option<String>,
  pub start_at: i32,
  pub options: Value,
}

pub async fn update(pool: &SqlitePool, id: Uuid, params: UpdateTaskParams) -> ServiceResult<Task> {
  ensure_task_exists(pool, id).await?;

  let task = update_task_row(pool, id, &params).await?;
  let project = get_project(pool, task.project_id).await?;

  Ok(build_task(task, project))
}

pub async fn run_task(pool: &SqlitePool, id: Uuid) -> ServiceResult<TaskRow> {
  update_task_status(pool, id, TaskStatus::InProgress).await
}

pub async fn failed_task(pool: &SqlitePool, id: Uuid) -> ServiceResult<TaskRow> {
  update_task_status(pool, id, TaskStatus::Failed).await
}

pub async fn completed_task(pool: &SqlitePool, id: Uuid) -> ServiceResult<TaskRow> {
  update_task_status(pool, id, TaskStatus::Finished).await
}

pub async fn schedule_task(pool: &SqlitePool, id: Uuid, start_at: i32) -> ServiceResult<TaskRow> {
  ensure_task_exists(pool, id).await?;

  sqlx::query_as::<_, TaskRow>(SCHEDULE_TASK)
    .bind(TaskStatus::New.to_string())
    .bind(start_at)
    .bind(id)
    .fetch_one(pool)
    .await
    .map_err(Into::into)
}

pub async fn delete(pool: &SqlitePool, id: Uuid) -> ServiceResult<()> {
  ensure_task_exists(pool, id).await?;

  sqlx::query(DELETE_TASK).bind(id).execute(pool).await?;

  Ok(())
}

pub async fn delete_completed_tasks(pool: &SqlitePool) -> ServiceResult<u64> {
  Ok(sqlx::query(DELETE_OLD_TASKS).execute(pool).await?.rows_affected())
}

pub async fn delete_by_update_date(pool: &SqlitePool) -> ServiceResult<()> {
  sqlx::query(DELETE_STALE_TASKS).execute(pool).await?;
  Ok(())
}

async fn create_task_row(pool: &SqlitePool, params: &CreateTaskParams) -> ServiceResult<TaskRow> {
  sqlx::query_as::<_, TaskRow>(INSERT_TASK)
    .bind(Uuid::new_v4())
    .bind(&params.r#type)
    .bind(params.project_id)
    .bind(&params.name)
    .bind(&params.external_id)
    .bind(&params.external_modified_at)
    .bind(&params.schedule)
    .bind(params.start_at)
    .bind(&params.options)
    .fetch_one(pool)
    .await
    .map_err(Into::into)
}

async fn update_task_row(pool: &SqlitePool, id: Uuid, params: &UpdateTaskParams) -> ServiceResult<TaskRow> {
  sqlx::query_as::<_, TaskRow>(UPDATE_TASK)
    .bind(&params.name)
    .bind(&params.schedule)
    .bind(params.start_at)
    .bind(&params.options)
    .bind(id)
    .fetch_one(pool)
    .await
    .map_err(Into::into)
}

async fn get_project(pool: &SqlitePool, project_id: Uuid) -> ServiceResult<ProjectRow> {
  sqlx::query_as::<_, ProjectRow>(FIND_PROJECT)
    .bind(project_id)
    .fetch_one(pool)
    .await
    .map_err(Into::into)
}

async fn ensure_task_exists(pool: &SqlitePool, id: Uuid) -> ServiceResult<()> {
  let exists = sqlx::query_as::<_, TaskRow>(FIND_TASK)
    .bind(id)
    .fetch_optional(pool)
    .await?;

  match exists {
    Some(_) => Ok(()),
    None => Err(ServiceError::ResourceNotFound(id.to_string())),
  }
}

async fn get_task_by_external_id(pool: &SqlitePool, external_id: &str) -> ServiceResult<Option<TaskRow>> {
  sqlx::query_as::<_, TaskRow>(FIND_TASK_BY_EXTARNAL_ID)
    .bind(external_id)
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

async fn update_task_status(pool: &SqlitePool, id: Uuid, status: TaskStatus) -> ServiceResult<TaskRow> {
  ensure_task_exists(pool, id).await?;

  sqlx::query_as::<_, TaskRow>(UPDATE_TASK_STATUS)
    .bind(status.to_string())
    .bind(id)
    .fetch_one(pool)
    .await
    .map_err(Into::into)
}

fn is_status_update_needed(
  existing_task: &TaskRow,
  existing_modified_at: DateTime<Utc>,
  task_modified_at: DateTime<Utc>,
) -> bool {
  existing_task
    .status
    .parse::<TaskStatus>()
    .map(|status| status == TaskStatus::Failed && task_modified_at > existing_modified_at)
    .unwrap_or(false)
}

fn build_task(task: TaskRow, project: ProjectRow) -> Task {
  Task {
    id: task.id,
    name: task.name,
    r#type: task.r#type,
    status: task.status,
    project,
    retries: task.retries,
    external_id: task.external_id,
    external_modified_at: task.external_modified_at,
    schedule: task.schedule,
    start_at: task.start_at,
    options: task.options,
    created_at: task.created_at,
    updated_at: task.updated_at,
  }
}
