use sqlx::{sqlite::SqliteRow, Row, SqlitePool};

use crate::ServiceResult;
use team_bot_entity::{
  project::{Project, ProjectRow},
  user::User,
};

const LIST_PROJECTS_QUERY: &str = r#"
  SELECT
    p.id as project_id,
    p.name as project_name,
    p.code as project_code,
    p.options as project_options,
    p.sync_with_exchange as project_sync_with_exchange,
    p.created_at as project_created_at,
    p.updated_at as project_updated_at,
    u.id as user_id,
    u.username as user_username,
    u.role as user_role,
    u.email as user_email,
    u.password as user_password,
    u.created_at as user_created_at,
    u.updated_at as user_updated_at
  FROM projects AS p
  LEFT OUTER JOIN users AS u ON p.owner_id = u.id
  ORDER BY p.id LIMIT ? OFFSET ?
"#;

/// Fetches a paginated list of projects with their associated users
///
/// # Arguments
/// * `pool` - The database connection pool
/// * `page` - The page number (1-based)
/// * `limit` - The number of items per page
///
/// # Returns
/// A tuple containing the projects and the total number of pages
pub async fn list(pool: &SqlitePool, page: i64, limit: i64) -> ServiceResult<(Vec<Project>, i64)> {
  let (total_count, projects) = tokio::try_join!(get_total_count(pool), fetch_projects(pool, page, limit))?;

  let total_pages = calculate_total_pages(total_count, limit);

  Ok((projects, total_pages))
}

/// Fetches a list of projects
///
/// # Arguments
/// * `pool` - The database connection pool
///
/// # Returns
/// A list containing the projects
pub async fn list_all(pool: &SqlitePool) -> ServiceResult<Vec<ProjectRow>> {
  sqlx::query_as::<_, ProjectRow>("SELECT * FROM projects ORDER BY id")
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

async fn fetch_projects(pool: &SqlitePool, page: i64, limit: i64) -> ServiceResult<Vec<Project>> {
  let offset = (page - 1) * limit;

  sqlx::query(LIST_PROJECTS_QUERY)
    .bind(limit)
    .bind(offset)
    .map(map_row_to_project)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

async fn get_total_count(pool: &SqlitePool) -> ServiceResult<i64> {
  let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM projects").fetch_one(pool).await?;
  Ok(count)
}

fn calculate_total_pages(total_count: i64, limit: i64) -> i64 {
  (total_count as f64 / limit as f64).ceil() as i64
}

fn map_row_to_project(row: SqliteRow) -> Project {
  Project {
    id: row.get("project_id"),
    name: row.get("project_name"),
    code: row.get("project_code"),
    sync_with_exchange: row.get("project_sync_with_exchange"),
    options: row.get("project_options"),
    owner: User {
      id: row.get("user_id"),
      username: row.get("user_username"),
      role: row.get("user_role"),
      email: row.get("user_email"),
      password: row.get("user_password"),
      created_at: row.get("user_created_at"),
      updated_at: row.get("user_updated_at"),
    },
    created_at: row.get("project_created_at"),
    updated_at: row.get("project_updated_at"),
  }
}
