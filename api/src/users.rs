use std::sync::Arc;

use axum::{
  extract::{Path, Query, State},
  http::{header, Response, StatusCode},
  middleware::from_fn_with_state,
  response::IntoResponse,
  Extension, Json,
};
use secrecy::SecretBox;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::SqlitePool;
use tower_cookies::{
  cookie::{time::Duration, SameSite},
  Cookie,
};
use tracing::{debug, instrument};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;
use validator::Validate;

use crate::{
  auth::{auth_guard, encode_jwt},
  ApiResult, AppJson,
};
use team_bot_entity::user::User;
use team_bot_service::mutation;
use team_bot_service::query;

const USERS_TAG: &str = "users";
const AUTH_COOKIE_NAME: &str = "token";
const DEFAULT_PAGE_SIZE: i64 = 10;

pub fn init_users_routes(state: Arc<SqlitePool>) -> OpenApiRouter<Arc<SqlitePool>> {
  let public_routes = OpenApiRouter::new().routes(routes!(login));

  let protected_auth_routes = OpenApiRouter::new()
    .routes(routes!(get_me, logout))
    .layer(from_fn_with_state(state.clone(), auth_guard));

  let protected_users_routes = OpenApiRouter::new()
    .routes(routes!(list_users, create_user, update_user, delete_user))
    .layer(from_fn_with_state(state.clone(), auth_guard));

  public_routes.merge(protected_auth_routes).merge(protected_users_routes)
}

#[derive(Debug, Deserialize, Validate, IntoParams)]
pub struct LoginParams {
  #[validate(length(min = 4))]
  username: String,
  #[validate(length(min = 1))]
  password: String,
}

#[derive(Debug, Serialize, ToSchema)]
struct LoginResponse {
  status: String,
  token: String,
}

#[utoipa::path(
  post,
  path = "/login",
  tag = USERS_TAG,
  params(
    LoginParams
  ),
  responses(
    (status = 200, description = "Login successful", body = LoginResponse),
    (status = 401, description = "Invalid credentials"),
    (status = 422, description = "Validation error")
  )
)]
#[instrument(skip(pool, input))]
async fn login(pool: State<Arc<SqlitePool>>, Json(input): Json<LoginParams>) -> ApiResult<impl IntoResponse> {
  input.validate()?;

  let params = mutation::users::LoginParams {
    username: input.username.to_ascii_lowercase(),
    password: SecretBox::new(Box::new(input.password)),
  };

  debug!("Try login user with params {:?}", params);

  let user = mutation::users::login(&pool, params).await?;
  let token = encode_jwt(user.id)?;

  let cookie = build_auth_cookie(token.clone(), true);
  let response = LoginResponse {
    status: "success".to_string(),
    token,
  };

  let mut response = Response::new(serde_json::to_string(&response).unwrap());
  response
    .headers_mut()
    .insert(header::SET_COOKIE, cookie.to_string().parse().unwrap());

  Ok(response)
}

#[utoipa::path(
  get,
  path = "/me",
  tag = USERS_TAG,
  responses(
    (status = OK, description = "Return current logged user", body = User),
    (status = 401, description = "Unauthorized")
  )
)]
async fn get_me(Extension(user): Extension<User>) -> ApiResult<Json<User>> {
  Ok(Json(user))
}

#[utoipa::path(
  post,
  path = "/logout",
  tag = USERS_TAG,
  responses(
    (status = 200, description = "Logout successful")
  )
)]
async fn logout() -> ApiResult<impl IntoResponse> {
  let cookie = build_auth_cookie("".to_string(), false);

  let mut response = Response::new(json!({"status": "success"}).to_string());
  response
    .headers_mut()
    .insert(header::SET_COOKIE, cookie.to_string().parse().unwrap());
  Ok(response)
}

#[derive(Debug, Deserialize, IntoParams)]
struct ListUsersParams {
  page: Option<i64>,
  users_per_page: Option<i64>,
}

#[utoipa::path(
  get,
  path = "",
  tag = USERS_TAG,
  params(
    ListUsersParams
  ),
  responses(
    (status = 200, description = "List all users successfully", body = [User]),
    (status = 401, description = "Unauthorized")
  )
)]
#[instrument(skip(pool))]
async fn list_users(
  State(pool): State<Arc<SqlitePool>>,
  Query(params): Query<ListUsersParams>,
) -> ApiResult<Json<Vec<User>>> {
  let page = params.page.unwrap_or(1);
  let users_per_page = params.users_per_page.unwrap_or(DEFAULT_PAGE_SIZE);

  let (users, _num_pages) = query::users::list(&pool, page, users_per_page).await?;

  Ok(Json(users))
}

#[derive(Debug, Validate, Deserialize, IntoParams)]
pub struct CreateUser {
  #[validate(length(min = 4))]
  username: String,
  #[validate(email)]
  email: String,
  #[validate(length(min = 8))]
  password: String,
}

#[utoipa::path(
  post,
  path = "",
  tag = USERS_TAG,
  params(
    CreateUser
  ),
  responses(
    (status = 201, description = "User created", body = User),
    (status = 401, description = "Unauthorized"),
    (status = 422, description = "Validation error")
  )
)]
#[instrument(skip(pool, input))]
async fn create_user(
  State(pool): State<Arc<SqlitePool>>,
  AppJson(input): AppJson<CreateUser>,
) -> ApiResult<Json<User>> {
  input.validate()?;

  let params = mutation::users::CreateUserParams {
    username: input.username,
    email: input.email,
    password: SecretBox::new(Box::new(input.password)),
  };

  debug!("Register new user with request: {:?}", params);

  let user = mutation::users::create(&pool, params).await?;

  Ok(Json(user))
}

#[derive(Debug, Validate, Deserialize, IntoParams)]
pub struct UpdateUser {
  #[validate(length(min = 4))]
  username: String,
  #[validate(email)]
  email: String,
  role: String,
  #[validate(length(min = 8))]
  password: String,
}

#[utoipa::path(
  put,
  path = "/{id}",
  tag = USERS_TAG,
  params(
    UpdateUser
  ),
  responses(
    (status = 200, description = "User updated", body = User),
    (status = 401, description = "Unauthorized"),
    (status = 404, description = "User not found"),
    (status = 422, description = "Validation error")
  )
)]
#[instrument(skip(pool, input))]
async fn update_user(
  State(pool): State<Arc<SqlitePool>>,
  Path(id): Path<Uuid>,
  Json(input): Json<UpdateUser>,
) -> ApiResult<Json<User>> {
  input.validate()?;

  let params = mutation::users::UpdateUserParams {
    username: input.username,
    role: input.role,
    email: input.email,
    password: SecretBox::new(Box::new(input.password)),
  };

  debug!("Update user with id {} and params {:?}", id, params);

  let user = mutation::users::update(&pool, id, params).await?;

  Ok(Json(user))
}

#[utoipa::path(
  delete,
  path = "/{id}",
  tag = USERS_TAG,
  responses(
    (status = 200, description = "User deleted"),
    (status = 401, description = "Unauthorized"),
    (status = 404, description = "User not found")
  ),
  params(
    ("id" = Uuid, Path, description = "User id")
  )
)]
#[instrument]
async fn delete_user(State(pool): State<Arc<SqlitePool>>, Path(id): Path<Uuid>) -> ApiResult<StatusCode> {
  mutation::users::delete(&pool, id).await?;

  Ok(StatusCode::OK)
}

fn build_auth_cookie(token: String, is_login: bool) -> Cookie<'static> {
  Cookie::build((AUTH_COOKIE_NAME, token))
    .path("/")
    .max_age(Duration::hours(if is_login { 24 } else { -1 }))
    .same_site(SameSite::Lax)
    .http_only(true)
    .build()
}
