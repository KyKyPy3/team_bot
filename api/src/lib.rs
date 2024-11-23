use std::{env, sync::Arc};

use axum::extract::{FromRequest, State};
use axum::http::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use axum::http::{HeaderValue, Method};
use axum::{response::IntoResponse, routing::get};
use error::ApiError;
use projects::init_projects_routes;
use serde_json::json;
use sqlx::SqlitePool;
use tasks::init_tasks_routes;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tower_cookies::CookieManagerLayer;
use tower_http::cors::CorsLayer;
use tower_http::services::{ServeDir, ServeFile};
use tracing::info;
use users::init_users_routes;
use utoipa::OpenApi;
use utoipa_axum::router::OpenApiRouter;
use utoipa_swagger_ui::SwaggerUi;

mod auth;
mod error;
mod projects;
mod tasks;
mod users;

const PLATFORM_BOT_TAG: &str = "platform_bot";

pub type ApiResult<T = ()> = Result<T, ApiError>;

#[derive(FromRequest)]
#[from_request(via(axum::Json), rejection(ApiError))]
struct AppJson<T>(T);

/// Handle health check requests
async fn health_handler(State(pool): State<Arc<SqlitePool>>) -> impl IntoResponse {
  let res = sqlx::query("SELECT 1").execute(&*pool).await;
  match res {
    Ok(_) => json!({
      "code": "200",
      "success": true,
    })
    .to_string(),
    Err(_) => json!({
      "code": "500",
      "success": false,
    })
    .to_string(),
  }
}

pub async fn run(state: Arc<SqlitePool>, cancel_token: CancellationToken) -> anyhow::Result<()> {
  let host = env::var("HOST").expect("HOST is not set in .env file");
  let port = env::var("PORT").expect("PORT is not set in .env file");
  let server_url = format!("{host}:{port}");

  let serve_dir = ServeDir::new("assets").not_found_service(ServeFile::new("assets/index.html"));

  // Initialize cors settings
  let cors = CorsLayer::new()
    .allow_origin("http://localhost:3000".parse::<HeaderValue>()?)
    .allow_methods([Method::GET, Method::POST, Method::PATCH, Method::DELETE])
    .allow_credentials(true)
    .allow_headers([AUTHORIZATION, ACCEPT, CONTENT_TYPE]);

  #[derive(OpenApi)]
  #[openapi(
    tags(
      (name = PLATFORM_BOT_TAG, description = "Bot management API")
    )
  )]
  struct ApiDoc;

  let (router, api) = OpenApiRouter::with_openapi(ApiDoc::openapi())
    .route("/health", get(health_handler))
    .nest("/api/users", init_users_routes(state.clone()))
    .nest("/api/projects", init_projects_routes(state.clone()))
    .nest("/api/tasks", init_tasks_routes(state.clone()))
    .nest_service("/assets", serve_dir.clone())
    .layer(CookieManagerLayer::new())
    .layer(cors)
    .fallback_service(serve_dir)
    .with_state(state)
    .split_for_parts();

  let router = router.merge(SwaggerUi::new("/swagger-ui").url("/api-docs/openapi.json", api.clone()));

  info!("Starting api server...");

  let listener = TcpListener::bind(&server_url).await?;
  axum::serve(listener, router.into_make_service())
    .with_graceful_shutdown(Box::pin(async move { cancel_token.cancelled().await }))
    .await?;

  info!("Stopped api server");

  Ok(())
}
