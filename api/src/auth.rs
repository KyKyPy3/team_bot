use std::sync::Arc;

use axum::{
  extract::{Request, State},
  http::{header, StatusCode},
  middleware::Next,
  response::IntoResponse,
  Json,
};
use axum_extra::extract::cookie::CookieJar;
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tracing::debug;
use uuid::Uuid;

use crate::error::ApiError;
use platform_bot_service::query;

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
  pub sub: String, // User associated with token
  pub iat: usize,  // Issued at time of the token
  pub exp: usize,  // Expiry time of the token
}

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
  pub status: &'static str,
  pub message: String,
}

pub static JWT_MAXAGE: Lazy<i64> = Lazy::new(|| {
  std::env::var("JWT_MAXAGE")
    .expect("JWT_MAXAGE must be set")
    .parse::<i64>()
    .unwrap()
});

pub static KEYS: Lazy<Keys> = Lazy::new(|| {
  let secret = std::env::var("JWT_SECRET").expect("JWT_SECRET must be set");
  Keys::new(secret.as_bytes())
});

pub struct Keys {
  pub encoding: EncodingKey,
  pub decoding: DecodingKey,
}

impl Keys {
  fn new(secret: &[u8]) -> Self {
    Self {
      encoding: EncodingKey::from_secret(secret),
      decoding: DecodingKey::from_secret(secret),
    }
  }
}

pub fn encode_jwt(user_id: Uuid) -> Result<String, ApiError> {
  let now = chrono::Utc::now();
  let iat = now.timestamp() as usize;
  let exp = (now + chrono::Duration::minutes(*JWT_MAXAGE)).timestamp() as usize;
  let claims: Claims = Claims {
    sub: user_id.to_string(),
    exp,
    iat,
  };

  encode(&Header::default(), &claims, &KEYS.encoding)
    .map_err(|_| ApiError::Anyhow(anyhow::anyhow!("Can't encode token")))
}

pub async fn auth_guard(
  cookie_jar: CookieJar,
  State(pool): State<Arc<SqlitePool>>,
  mut req: Request,
  next: Next,
) -> Result<impl IntoResponse, (StatusCode, Json<ErrorResponse>)> {
  let token = cookie_jar
    .get("token")
    .map(|cookie| cookie.value().to_string())
    .or_else(|| {
      req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|auth_header| auth_header.to_str().ok())
        .and_then(|auth_value| {
          if auth_value.starts_with("Bearer ") {
            Some(auth_value[7..].to_owned())
          } else {
            None
          }
        })
    });

  let token = token.ok_or_else(|| {
    let json_error = ErrorResponse {
      status: "fail",
      message: "You are not logged in, please provide token".to_string(),
    };
    (StatusCode::UNAUTHORIZED, Json(json_error))
  })?;

  let claims = decode::<Claims>(&token, &KEYS.decoding, &Validation::default())
    .map_err(|_| {
      let json_error = ErrorResponse {
        status: "fail",
        message: "Invalid token".to_string(),
      };
      (StatusCode::UNAUTHORIZED, Json(json_error))
    })?
    .claims;

  let user = query::users::find_by_id(&pool.clone(), Uuid::parse_str(&claims.sub).unwrap())
    .await
    .map_err(|_| {
      let json_error = ErrorResponse {
        status: "fail",
        message: "You are not logged in, please provide token".to_string(),
      };
      (StatusCode::UNAUTHORIZED, Json(json_error))
    })?
    .ok_or({
      let json_error = ErrorResponse {
        status: "fail",
        message: "The user belonging to this token no longer exists".to_string(),
      };
      (StatusCode::UNAUTHORIZED, Json(json_error))
    })?;

  debug!("fetch user model from db {:?}", user);

  req.extensions_mut().insert(user);
  Ok(next.run(req).await)
}
