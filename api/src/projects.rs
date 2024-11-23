use std::sync::Arc;

use axum::{
  extract::{Path, Query, State},
  middleware::from_fn_with_state,
  Json,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::SqlitePool;
use tracing::{debug, instrument};
use utoipa::IntoParams;
use utoipa_axum::{
  router::{OpenApiRouter, UtoipaMethodRouterExt},
  routes,
};
use uuid::Uuid;
use validator::Validate;

use crate::{auth::auth_guard, ApiResult, AppJson};
use platform_bot_entity::project::Project;
use platform_bot_service::{mutation, query};

const PROJECTS_TAG: &str = "projects";
const DEFAULT_PAGE: i64 = 1;
const DEFAULT_PROJECTS_PER_PAGE: i64 = 5;

pub fn init_projects_routes(state: Arc<SqlitePool>) -> OpenApiRouter<Arc<SqlitePool>> {
  OpenApiRouter::new().routes(
    routes!(list_projects, create_project, update_project, delete_project)
      .layer(from_fn_with_state(state.clone(), auth_guard)),
  )
}

#[derive(Debug, Deserialize, IntoParams)]
struct ListProjectsParams {
  page: Option<i64>,
  projects_per_page: Option<i64>,
}

#[utoipa::path(
  get,
  path = "",
  tag = PROJECTS_TAG,
  params(
    ListProjectsParams
  ),
  responses(
    (status = 200, description = "List all projects successfully", body = [Project])
  )
)]
#[instrument(skip(pool))]
async fn list_projects(
  State(pool): State<Arc<SqlitePool>>,
  Query(params): Query<ListProjectsParams>,
) -> ApiResult<Json<Vec<Project>>> {
  let page = params.page.unwrap_or(DEFAULT_PAGE);
  let projects_per_page = params.projects_per_page.unwrap_or(DEFAULT_PROJECTS_PER_PAGE);

  let (projects, _num_pages) = query::projects::list(&pool, page, projects_per_page).await?;

  Ok(Json(projects))
}

#[derive(Debug, Validate, Deserialize, Serialize, IntoParams)]
pub struct CreateProject {
  #[validate(length(min = 4))]
  name: String,
  #[validate(length(min = 2, max = 4))]
  code: String,
  owner: Uuid,
  options: Option<Value>,
}

#[utoipa::path(
  post,
  path = "",
  tag = PROJECTS_TAG,
  params(
    CreateProject
  ),
  responses(
    (status = 201, description = "Project created successfully", body = Project),
  )
)]
async fn create_project(
  State(pool): State<Arc<SqlitePool>>,
  AppJson(input): AppJson<CreateProject>,
) -> ApiResult<Json<Project>> {
  debug!("Register new project with request: {:?}", input);

  input.validate()?;

  let project = mutation::projects::create(
    &pool,
    mutation::projects::CreateProjectParams {
      name: input.name,
      code: input.code,
      owner_id: input.owner,
      options: input.options,
    },
  )
  .await?;

  Ok(Json(project))
}

#[derive(Debug, Validate, Deserialize, Serialize, IntoParams)]
pub struct UpdateProject {
  #[validate(length(min = 4))]
  name: String,
  #[validate(length(min = 2, max = 4))]
  code: String,
  options: Option<Value>,
}

#[utoipa::path(
  put,
  path = "/{id}",
  tag = PROJECTS_TAG,
  params(
    UpdateProject
  ),
  responses(
    (status = 200, description = "Project updated successfully", body = Project),
  )
)]
#[instrument(skip(pool), fields(project_id = %id))]
async fn update_project(
  State(pool): State<Arc<SqlitePool>>,
  Path(id): Path<Uuid>,
  Json(input): Json<UpdateProject>,
) -> ApiResult<Json<Project>> {
  debug!("Update project with id {} and params {:?}", id, input);

  input.validate()?;

  let project = mutation::projects::update(
    &pool,
    id,
    mutation::projects::UpdateProjectParams {
      name: input.name,
      code: input.code,
      options: input.options,
    },
  )
  .await?;

  Ok(Json(project))
}

#[utoipa::path(
  delete,
  path = "/{id}",
  tag = PROJECTS_TAG,
  responses(
    (status = 200, description = "Project successfully deleted"),
  ),
  params(
    ("id" = Uuid, Path, description = "Project id")
  )
)]
#[instrument(skip(pool), fields(project_id = %id))]
async fn delete_project(State(pool): State<Arc<SqlitePool>>, Path(id): Path<Uuid>) -> ApiResult<()> {
  debug!("Remove project with id {}", id);

  mutation::projects::delete(&pool, id).await?;

  Ok(())
}
