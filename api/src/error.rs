use axum::extract::rejection::JsonRejection;
use axum::response::{IntoResponse, Response};
use axum::{http::StatusCode, Json};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use platform_bot_service::error::ServiceError;

#[derive(Debug, Error)]
pub enum ApiError {
  #[error(transparent)]
  SrvError(#[from] ServiceError),
  #[error(transparent)]
  JsonRejection(JsonRejection),
  #[error(transparent)]
  InvalidInputError(#[from] validator::ValidationErrors),
  #[error("Invalid schedule format: {0}")]
  InvalidSchedule(String),
  #[error("Failed to calculate next run time: {0}")]
  ScheduleCalculation(String),
  #[error("an internal server error occurred")]
  Anyhow(#[from] anyhow::Error),
}

impl ApiError {
  pub fn response(self) -> (StatusCode, AppResponseError) {
    use ApiError::*;
    let message = self.to_string();

    let (kind, code, details, status_code) = match self {
      JsonRejection(rejection) => (
        "INVALID_INPUT_ERROR".to_string(),
        None,
        vec![(rejection.status().to_string(), vec![rejection.body_text()])],
        StatusCode::BAD_REQUEST,
      ),
      SrvError(ServiceError::InvalidCredentials()) => (
        "INVALID_CREDENTIALS".to_string(),
        None,
        vec![],
        StatusCode::UNAUTHORIZED,
      ),
      SrvError(ServiceError::ResourceNotFound(_)) => {
        ("RESOURCE_NOT_FOUND".to_string(), None, vec![], StatusCode::NOT_FOUND)
      },
      SrvError(_err) => (
        "INTERNAL_SERVER_ERROR".to_string(),
        None,
        vec![],
        StatusCode::INTERNAL_SERVER_ERROR,
      ),
      InvalidInputError(err) => (
        "INVALID_INPUT_ERROR".to_string(),
        None,
        err
          .field_errors()
          .into_iter()
          .map(|(p, e)| {
            (
              p.to_string(),
              e.iter().map(|err| err.code.to_string()).collect::<Vec<String>>(),
            )
          })
          .collect(),
        StatusCode::BAD_REQUEST,
      ),
      InvalidSchedule(err) => (
        "INTERNAL_SERVER_ERROR".to_string(),
        None,
        vec![],
        StatusCode::INTERNAL_SERVER_ERROR,
      ),
      ScheduleCalculation(err) => (
        "INTERNAL_SERVER_ERROR".to_string(),
        None,
        vec![],
        StatusCode::INTERNAL_SERVER_ERROR,
      ),
      Anyhow(ref e) => {
        tracing::error!("Generic error: {:?}", e);

        (
          "INTERNAL_SERVER_ERROR".to_string(),
          None,
          vec![],
          StatusCode::INTERNAL_SERVER_ERROR,
        )
      },
    };

    (status_code, AppResponseError::new(kind, message, code, details))
  }
}

impl IntoResponse for ApiError {
  fn into_response(self) -> Response {
    let (status_code, body) = self.response();
    (status_code, Json(body)).into_response()
  }
}

impl From<JsonRejection> for ApiError {
  fn from(rejection: JsonRejection) -> Self {
    Self::JsonRejection(rejection)
  }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct AppResponseError {
  pub kind: String,
  pub error_message: String,
  pub code: Option<i32>,
  pub details: Vec<(String, Vec<String>)>,
}

impl AppResponseError {
  pub fn new(
    kind: impl Into<String>,
    message: impl Into<String>,
    code: Option<i32>,
    details: Vec<(String, Vec<String>)>,
  ) -> Self {
    Self {
      kind: kind.into(),
      error_message: message.into(),
      code,
      details,
    }
  }
}
