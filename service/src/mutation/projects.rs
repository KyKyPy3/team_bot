use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::{error::ServiceError, ServiceResult};
use team_bot_entity::{
  project::{Project, ProjectRow},
  user::User,
};

// SQL Query Constants
const FIND_PROJECT_BY_CODE: &str = "SELECT * FROM projects WHERE code = ?1";
const FIND_PROJECT_BY_ID: &str = "SELECT * FROM projects WHERE id = ?1";
const FIND_USER: &str = "SELECT * FROM users WHERE id = ?1";
const INSERT_PROJECT: &str = r#"
    INSERT INTO projects (id, name, code, owner_id, options)
    VALUES (?1, ?2, ?3, ?4, ?5)
    RETURNING *
"#;
const UPDATE_PROJECT: &str = r#"
    UPDATE projects
    SET name = ?1, code = ?2, options = ?3
    WHERE id = ?4
    RETURNING *
"#;
const DELETE_PROJECT: &str = "DELETE FROM projects WHERE id = ?";

#[derive(Debug, Deserialize)]
pub struct CreateProjectParams {
  pub name: String,
  pub code: String,
  pub owner_id: Uuid,
  pub options: Option<Value>,
}

/// Creates a new project with the given parameters
///
/// # Errors
/// - ProjectAlreadyExist if a project with the same code exists
/// - DatabaseError for any database-related issues
pub async fn create(pool: &SqlitePool, params: CreateProjectParams) -> ServiceResult<Project> {
  ensure_project_not_exists(pool, &params.code).await?;

  let project = create_project_row(pool, &params).await?;
  let owner = get_user(pool, params.owner_id).await?;

  Ok(build_project(project, owner))
}

#[derive(Debug, Deserialize, Clone)]
pub struct UpdateProjectParams {
  pub name: String,
  pub code: String,
  pub options: Option<Value>,
}

/// Updates an existing project by ID
///
/// # Errors
/// - ResourceNotFound if project doesn't exist
/// - DatabaseError for any database-related issues
pub async fn update(pool: &SqlitePool, id: Uuid, params: UpdateProjectParams) -> ServiceResult<Project> {
  let existing = get_project(pool, id).await?;

  let project = update_project_row(pool, id, params, existing.options).await?;
  let owner = get_user(pool, project.owner_id).await?;

  Ok(build_project(project, owner))
}

/// Deletes a project by ID
///
/// # Errors
/// - ResourceNotFound if project doesn't exist
/// - DatabaseError for any database-related issues
pub async fn delete(pool: &SqlitePool, id: Uuid) -> ServiceResult<()> {
  ensure_project_exists(pool, id).await?;

  sqlx::query(DELETE_PROJECT).bind(id).execute(pool).await?;

  Ok(())
}

async fn ensure_project_not_exists(pool: &SqlitePool, code: &str) -> ServiceResult<()> {
  let exists = sqlx::query_as::<_, ProjectRow>(FIND_PROJECT_BY_CODE)
    .bind(code)
    .fetch_optional(pool)
    .await
    .map_err(ServiceError::DatabaseError)?;

  match exists {
    Some(_) => Err(ServiceError::ProjectAlreadyExist(code.to_string())),
    None => Ok(()),
  }
}

async fn ensure_project_exists(pool: &SqlitePool, id: Uuid) -> ServiceResult<()> {
  get_project(pool, id).await.map(|_| ())
}

async fn get_project(pool: &SqlitePool, id: Uuid) -> ServiceResult<ProjectRow> {
  sqlx::query_as::<_, ProjectRow>(FIND_PROJECT_BY_ID)
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| ServiceError::ResourceNotFound(id.to_string()))
}

async fn get_user(pool: &SqlitePool, user_id: Uuid) -> ServiceResult<User> {
  sqlx::query_as::<_, User>(FIND_USER)
    .bind(user_id)
    .fetch_one(pool)
    .await
    .map_err(Into::into)
}

async fn create_project_row(pool: &SqlitePool, params: &CreateProjectParams) -> ServiceResult<ProjectRow> {
  sqlx::query_as::<_, ProjectRow>(INSERT_PROJECT)
    .bind(Uuid::new_v4())
    .bind(&params.name)
    .bind(&params.code)
    .bind(params.owner_id)
    .bind(params.options.clone().unwrap_or_else(|| json!({})))
    .fetch_one(pool)
    .await
    .map_err(Into::into)
}

async fn update_project_row(
  pool: &SqlitePool,
  id: Uuid,
  params: UpdateProjectParams,
  existing_options: Value,
) -> ServiceResult<ProjectRow> {
  sqlx::query_as::<_, ProjectRow>(UPDATE_PROJECT)
    .bind(&params.name)
    .bind(&params.code)
    .bind(params.options.unwrap_or(existing_options))
    .bind(id)
    .fetch_one(pool)
    .await
    .map_err(Into::into)
}

fn build_project(project: ProjectRow, owner: User) -> Project {
  Project {
    id: project.id,
    name: project.name,
    code: project.code,
    options: project.options,
    sync_with_exchange: project.sync_with_exchange,
    owner,
    created_at: project.created_at,
    updated_at: project.updated_at,
  }
}
