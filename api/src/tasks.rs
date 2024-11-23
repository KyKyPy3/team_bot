use std::{str::FromStr, sync::Arc};

use anyhow::Result;
use axum::{
  extract::{Path, Query, State},
  middleware::{self, from_fn_with_state},
  Json,
};
use chrono::{DateTime, FixedOffset, Utc};
use cron::Schedule;
use duration_str::parse;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tracing::{debug, instrument};
use utoipa::IntoParams;
use utoipa_axum::{
  router::{OpenApiRouter, UtoipaMethodRouterExt},
  routes,
};
use uuid::Uuid;
use validator::Validate;

use crate::{auth::auth_guard, error::ApiError, ApiResult, AppJson};
use team_bot_entity::task::Task;
use team_bot_service::{mutation, query};

const TASKS_TAG: &str = "tasks";
const DEFAULT_PAGE: i64 = 1;
const DEFAULT_TASKS_PER_PAGE: i64 = 5;
const EVERY_PREFIX: &str = "@every ";

pub fn init_tasks_routes(state: Arc<SqlitePool>) -> OpenApiRouter<Arc<SqlitePool>> {
  OpenApiRouter::new()
    .routes(
      routes!(list_tasks, create_task, update_task, delete_task).layer(from_fn_with_state(state.clone(), auth_guard)),
    )
    .route_layer(middleware::from_fn_with_state(state.clone(), auth_guard))
}

#[derive(Debug, Deserialize, IntoParams)]
struct ListTasksParams {
  page: Option<i64>,
  tasks_per_page: Option<i64>,
}

#[utoipa::path(
  get,
  path = "",
  tag = TASKS_TAG,
  params(
    ListTasksParams
  ),
  responses(
    (status = 200, description = "List all tasks successfully", body = [Task])
  )
)]
#[instrument(skip(pool))]
async fn list_tasks(
  State(pool): State<Arc<SqlitePool>>,
  Query(params): Query<ListTasksParams>,
) -> ApiResult<Json<Vec<Task>>> {
  let page = params.page.unwrap_or(DEFAULT_PAGE);
  let tasks_per_page = params.tasks_per_page.unwrap_or(DEFAULT_TASKS_PER_PAGE);

  let (tasks, _num_pages) = query::tasks::list(&pool, page, tasks_per_page).await?;

  Ok(Json(tasks))
}

#[derive(Debug, Validate, Deserialize, Serialize, IntoParams)]
pub struct CreateTask {
  #[validate(length(min = 4))]
  name: String,
  r#type: String,
  schedule: Option<String>,
  project_id: Uuid,
  start_at: DateTime<FixedOffset>,
  options: serde_json::Value,
}

#[utoipa::path(
  post,
  path = "",
  tag = TASKS_TAG,
  params(
    CreateTask
  ),
  responses(
    (status = 201, description = "Task created successfully", body = Task),
  )
)]
#[instrument(skip(pool, input))]
async fn create_task(
  State(pool): State<Arc<SqlitePool>>,
  AppJson(input): AppJson<CreateTask>,
) -> ApiResult<Json<Task>> {
  debug!("Register new task with request: {:?}", input);

  input.validate()?;

  let start_at = calculate_next_execution_time(input.schedule.as_ref(), input.start_at)?;

  let task = mutation::tasks::create(
    &pool,
    mutation::tasks::CreateTaskParams {
      name: input.name,
      r#type: input.r#type,
      project_id: input.project_id,
      external_id: None,
      external_modified_at: None,
      schedule: input.schedule,
      start_at,
      options: input.options,
    },
  )
  .await?;

  Ok(Json(task))
}

#[derive(Debug, Validate, Deserialize, Serialize, IntoParams)]
pub struct UpdateTask {
  #[validate(length(min = 4))]
  name: String,
  schedule: Option<String>,
  start_at: DateTime<FixedOffset>,
  options: serde_json::Value,
}

#[utoipa::path(
  put,
  path = "/{id}",
  tag = TASKS_TAG,
  params(
    UpdateTask
  ),
  responses(
    (status = 200, description = "Task updated successfully", body = Task),
  )
)]
#[instrument(skip(pool), fields(task_id = %id))]
async fn update_task(
  State(pool): State<Arc<SqlitePool>>,
  Path(id): Path<Uuid>,
  Json(input): Json<UpdateTask>,
) -> ApiResult<Json<Task>> {
  debug!("Update task with id {} and params {:?}", id, input);

  input.validate()?;

  let start_at = calculate_next_execution_time(input.schedule.as_ref(), input.start_at)?;

  let task = mutation::tasks::update(
    &pool,
    id,
    mutation::tasks::UpdateTaskParams {
      name: input.name,
      schedule: input.schedule,
      start_at,
      options: input.options,
    },
  )
  .await?;

  Ok(Json(task))
}

#[utoipa::path(
  delete,
  path = "/{id}",
  tag = TASKS_TAG,
  responses(
    (status = 200, description = "Task successfully deleted"),
  ),
  params(
    ("id" = Uuid, Path, description = "Task id")
  )
)]
#[instrument(skip(pool), fields(task_id = %id))]
async fn delete_task(State(pool): State<Arc<SqlitePool>>, Path(id): Path<Uuid>) -> ApiResult<()> {
  debug!("Remove task with id {}", id);

  mutation::tasks::delete(&pool, id).await?;

  Ok(())
}

fn calculate_next_execution_time(schedule: Option<&String>, start_at: DateTime<FixedOffset>) -> Result<i32> {
  let current_time = Utc::now().timestamp();
  let start_timestamp = start_at.to_utc().timestamp();

  if start_timestamp >= current_time {
    return Ok(start_timestamp as i32);
  }

  let Some(schedule) = schedule else {
    return Ok(start_timestamp as i32);
  };

  if schedule.starts_with(EVERY_PREFIX) {
    calculate_interval_based_time(schedule, start_timestamp)
  } else {
    calculate_cron_based_time(schedule, start_at)
  }
}

fn calculate_interval_based_time(schedule: &str, start_timestamp: i64) -> Result<i32> {
  let duration_str = schedule.trim_start_matches(EVERY_PREFIX);
  let duration = parse(duration_str).map_err(|e| ApiError::InvalidSchedule(e.to_string()))?;

  let interval = chrono::Duration::from_std(duration).map_err(|e| ApiError::ScheduleCalculation(e.to_string()))?;

  Ok((start_timestamp + interval.num_seconds()) as i32)
}

fn calculate_cron_based_time(schedule: &str, start_at: DateTime<FixedOffset>) -> Result<i32> {
  let schedule = Schedule::from_str(schedule).map_err(|e| ApiError::InvalidSchedule(e.to_string()))?;

  let next_run = schedule
    .after(&start_at.to_utc())
    .next()
    .ok_or_else(|| ApiError::ScheduleCalculation("Failed to calculate next run".into()))?;

  Ok(next_run.timestamp() as i32)
}
