use sqlx::{sqlite::SqliteRow, Row, SqlitePool};

use crate::ServiceResult;
use platform_bot_entity::{project::ProjectRow, task::Task};

const LIST_TASKS_QUERY: &str = r#"
  SELECT
    p.id as project_id,
    p.name as project_name,
    p.code as project_code,
    p.options as project_options,
    p.sync_with_exchange as project_sync_with_exchange,
    p.owner_id as project_owner_id,
    p.created_at as project_created_at,
    p.updated_at as project_updated_at,
    t.id as task_id,
    t.type as task_type,
    t.status as task_status,
    t.options as task_options,
    t.start_at as task_start_at,
    t.schedule as task_schedule,
    t.name as task_name,
    t.retries as task_retries,
    t.external_id as task_external_id,
    t.created_at as task_created_at,
    t.updated_at as task_updated_at
  FROM tasks AS t
  LEFT OUTER JOIN projects AS p ON t.project_id = p.id
  ORDER BY t.id LIMIT ? OFFSET ?
"#;

const RUNNABLE_TASKS_QUERY: &str = r#"
  SELECT
    p.id as project_id,
    p.name as project_name,
    p.code as project_code,
    p.options as project_options,
    p.sync_with_exchange as project_sync_with_exchange,
    p.owner_id as project_owner_id,
    p.created_at as project_created_at,
    p.updated_at as project_updated_at,
    t.id as task_id,
    t.type as task_type,
    t.status as task_status,
    t.options as task_options,
    t.start_at as task_start_at,
    t.schedule as task_schedule,
    t.name as task_name,
    t.retries as task_retries,
    t.external_id as task_external_id,
    t.created_at as task_created_at,
    t.updated_at as task_updated_at
  FROM tasks AS t
  LEFT OUTER JOIN projects AS p ON t.project_id = p.id
  WHERE status NOT IN ('finished', 'in_progress', 'failed')
  AND start_at <= unixepoch()
  ORDER BY t.id
"#;

/// Fetches a paginated list of tasks with their associated projects
///
/// # Arguments
/// * `pool` - The database connection pool
/// * `page` - The page number (1-based)
/// * `limit` - The number of items per page
///
/// # Returns
/// A tuple containing the tasks and the total number of pages
pub async fn list(pool: &SqlitePool, page: i64, limit: i64) -> ServiceResult<(Vec<Task>, i64)> {
  let (total_count, tasks) = tokio::try_join!(get_total_count(pool), fetch_paginated_tasks(pool, page, limit))?;

  let total_pages = calculate_total_pages(total_count, limit);
  Ok((tasks, total_pages))
}

/// Fetches a list of tasks that ready to run
///
/// # Arguments
/// * `pool` - The database connection pool
///
/// # Returns
/// A list containing the tasks to run
pub async fn get_tasks_to_run(pool: &SqlitePool) -> ServiceResult<Vec<Task>> {
  sqlx::query(RUNNABLE_TASKS_QUERY)
    .map(map_task)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

async fn fetch_paginated_tasks(pool: &SqlitePool, page: i64, limit: i64) -> ServiceResult<Vec<Task>> {
  let offset = (page - 1) * limit;

  sqlx::query(LIST_TASKS_QUERY)
    .bind(limit)
    .bind(offset)
    .map(map_task)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

async fn get_total_count(pool: &SqlitePool) -> ServiceResult<i64> {
  let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM tasks").fetch_one(pool).await?;
  Ok(count)
}

fn calculate_total_pages(total_count: i64, limit: i64) -> i64 {
  (total_count as f64 / limit as f64).ceil() as i64
}

fn map_task(row: SqliteRow) -> Task {
  Task {
    id: row.get("task_id"),
    r#type: row.get("task_type"),
    status: row.get("task_status"),
    options: row.get("task_options"),
    start_at: row.get("task_start_at"),
    schedule: row.get("task_schedule"),
    name: row.get("task_name"),
    retries: row.get("task_retries"),
    external_id: row.get("task_external_id"),
    project: map_project_row(&row),
    created_at: row.get("task_created_at"),
    updated_at: row.get("task_updated_at"),
  }
}

fn map_project_row(row: &SqliteRow) -> ProjectRow {
  ProjectRow {
    id: row.get("project_id"),
    name: row.get("project_name"),
    code: row.get("project_code"),
    options: row.get("project_options"),
    sync_with_exchange: row.get("project_sync_with_exchange"),
    owner_id: row.get("project_owner_id"),
    created_at: row.get("project_created_at"),
    updated_at: row.get("project_updated_at"),
  }
}
