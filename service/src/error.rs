use curl::Error as CurlError;
use sqlx::Error as SqlxError;
use std::string::FromUtf8Error;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ServiceError {
  #[error("Database error: {0}")]
  DatabaseError(#[from] SqlxError),

  #[error("Entity `{0}` is not found")]
  ResourceNotFound(String),

  #[error("Invalid credentials")]
  InvalidCredentials(),

  #[error("User with email `{0}` already exists")]
  UserAlreadyExist(String),

  #[error("Project with code `{0}` already exists")]
  ProjectAlreadyExist(String),

  #[error("Exchange API error: {status_code} - {message}")]
  ExchangeApiError { status_code: u32, message: String },

  #[error("Exchange request configuration error: {0}")]
  ExchangeRequestConfigError(String),

  #[error("Exchange connection error: {0}")]
  ExchangeConnectionError(#[from] CurlError),

  #[error("Invalid event format: {0}")]
  InvalidEventFormat(String),

  #[error("Invalid schedule format: {0}")]
  InvalidScheduleFormat(String),

  #[error("Parsing error: {0}")]
  ParseError(String),

  #[error("Environment variable not set: {0}")]
  EnvVarError(String),

  #[error("Encoding error: {0}")]
  EncodingError(#[from] FromUtf8Error),

  #[error("Serialization error: {0}")]
  SerializationError(#[from] serde_json::Error),

  #[error(transparent)]
  InternalError(#[from] anyhow::Error),
}

impl ServiceError {
  pub fn exchange_api_error(status_code: u32, message: impl Into<String>) -> Self {
    Self::ExchangeApiError {
      status_code,
      message: message.into(),
    }
  }

  pub fn invalid_event(reason: impl Into<String>) -> Self {
    Self::InvalidEventFormat(reason.into())
  }

  pub fn invalid_schedule(reason: impl Into<String>) -> Self {
    Self::InvalidScheduleFormat(reason.into())
  }

  pub fn config_error(reason: impl Into<String>) -> Self {
    Self::ExchangeRequestConfigError(reason.into())
  }

  pub fn parse_error(reason: impl Into<String>) -> Self {
    Self::ParseError(reason.into())
  }

  pub fn env_var_missing(var_name: impl Into<String>) -> Self {
    Self::EnvVarError(var_name.into())
  }
}
