use anyhow::Context;
use argon2::password_hash::SaltString;
use argon2::{Algorithm, Argon2, Params, PasswordHash, PasswordHasher, PasswordVerifier, Version};
use rand_core::OsRng;
use secrecy::{ExposeSecret, SecretBox};
use serde::Deserialize;
use sqlx::SqlitePool;
use tokio::task;
use tracing::{error, info};
use uuid::Uuid;

use crate::{error::ServiceError, ServiceResult};
use platform_bot_entity::user::User;

const FIND_USER_BY_EMAIL: &str = "SELECT * FROM users WHERE email = ?1";
const FIND_USER_BY_USERNAME: &str = "SELECT * FROM users WHERE username = ?1";
const FIND_USER_BY_ID: &str = "SELECT * FROM users WHERE id = ?1";
const CREATE_USER: &str = "INSERT INTO users (id, username, email, password) VALUES (?1, ?2, ?3, ?4) RETURNING *";
const UPDATE_USER: &str =
  "UPDATE users SET username = ?1, role = ?2, email = ?3, password = ?4 WHERE id = ?5 RETURNING *";
const DELETE_USER: &str = "DELETE FROM users WHERE id = ?";

#[derive(Debug, Deserialize)]
pub struct LoginParams {
  pub username: String,
  pub password: SecretBox<String>,
}

pub async fn login(pool: &SqlitePool, params: LoginParams) -> ServiceResult<User> {
  let user = find_user_by_username(pool, &params.username).await?;
  verify_password(SecretBox::from(Box::new(user.password.to_owned())), params.password).await?;
  Ok(user)
}

#[derive(Debug, Deserialize)]
pub struct CreateUserParams {
  pub username: String,
  pub email: String,
  pub password: SecretBox<String>,
}

pub async fn create(pool: &SqlitePool, mut params: CreateUserParams) -> ServiceResult<User> {
  // Check if user already exists
  if let Some(_) = check_user_exists(pool, &params.email).await? {
    return Err(ServiceError::UserAlreadyExist(params.email));
  }

  let password = std::mem::take(&mut params.password);
  let hashed_password = hash_password(password).await?;
  create_new_user(pool, params, &hashed_password).await
}

#[derive(Debug, Deserialize)]
pub struct UpdateUserParams {
  pub username: String,
  pub role: String,
  pub email: String,
  pub password: SecretBox<String>,
}

/// Updates an existing user's information in the database
///
/// # Arguments
///
/// * `pool` - A SQLite connection pool for database operations
/// * `id` - The UUID of the user to update
/// * `params` - The new user information containing:
///   - username: New username for the user
///   - role: New role assignment
///   - email: New email address
///   - password: New password (will be hashed before storage)
///
/// # Returns
///
/// Returns a `ServiceResult<User>` which is:
/// * `Ok(User)` - Successfully updated user with the new information
/// * `Err(ServiceError)` - If any of the following occurs:
///   - User not found (ResourceNotFound)
///   - Database error
///   - Password hashing failure
///
/// # Example
///
/// ```rust
/// let params = UpdateUserParams {
///     username: "new_username".to_string(),
///     role: "admin".to_string(),
///     email: "new.email@example.com".to_string(),
///     password: SecretBox::new("new_password".to_string()),
/// };
///
/// match update(&pool, user_id, params).await {
///     Ok(updated_user) => println!("User updated successfully"),
///     Err(e) => eprintln!("Failed to update user: {}", e),
/// }
/// ```
///
/// # Security Considerations
///
/// - Passwords are hashed using Argon2 before storage
/// - The original password is securely cleared from memory after hashing
/// - Database operations are performed using parameterized queries to prevent SQL injection
pub async fn update(pool: &SqlitePool, id: Uuid, mut params: UpdateUserParams) -> ServiceResult<User> {
  // Verify user exists
  ensure_user_exists(pool, id).await?;

  let password = std::mem::take(&mut params.password);
  let hashed_password = hash_password(password).await?;
  update_existing_user(pool, id, params, &hashed_password).await
}

/// Deletes a user from the database by their ID
///
/// # Arguments
///
/// * `pool` - A SQLite connection pool for database operations
/// * `id` - The UUID of the user to delete
///
/// # Returns
///
/// Returns a `ServiceResult<()>` which is:
/// * `Ok(())` - User was successfully deleted
/// * `Err(ServiceError)` - If any of the following occurs:
///   - User not found (ResourceNotFound)
///   - Database error during deletion
///
/// # Example
///
/// ```rust
/// let user_id = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000")?;
///
/// match delete(&pool, user_id).await {
///     Ok(()) => println!("User deleted successfully"),
///     Err(e) => eprintln!("Failed to delete user: {}", e),
/// }
/// ```
///
/// # Notes
///
/// - This operation is irreversible
/// - Ensures the user exists before attempting deletion
/// - The deletion is performed atomically
/// - Related data might need to be handled separately depending on foreign key constraints
pub async fn delete(pool: &SqlitePool, id: Uuid) -> ServiceResult<()> {
  ensure_user_exists(pool, id).await?;

  sqlx::query(DELETE_USER).bind(id).execute(pool).await?;

  Ok(())
}

async fn create_new_user(pool: &SqlitePool, params: CreateUserParams, hashed_password: &str) -> ServiceResult<User> {
  sqlx::query_as::<_, User>(CREATE_USER)
    .bind(Uuid::new_v4())
    .bind(params.username)
    .bind(params.email)
    .bind(hashed_password)
    .fetch_one(pool)
    .await
    .map_err(Into::into)
}

async fn update_existing_user(
  pool: &SqlitePool,
  id: Uuid,
  params: UpdateUserParams,
  hashed_password: &str,
) -> ServiceResult<User> {
  sqlx::query_as::<_, User>(UPDATE_USER)
    .bind(params.username)
    .bind(params.role)
    .bind(params.email)
    .bind(hashed_password)
    .bind(id)
    .fetch_one(pool)
    .await
    .map_err(Into::into)
}

async fn hash_password(password: SecretBox<String>) -> ServiceResult<String> {
  task::spawn_blocking(move || {
    let salt = SaltString::generate(&mut OsRng);
    let argon2_config = Argon2::new(
      Algorithm::Argon2id,
      Version::V0x13,
      Params::new(15000, 2, 1, None).unwrap(),
    );

    argon2_config
      .hash_password(password.expose_secret().as_bytes(), &salt)
      .map_err(|err| {
        error!("Failed to hash password: {}", err);
        ServiceError::InvalidCredentials()
      })
      .map(|hash| hash.to_string())
  })
  .await
  .context("panic in hash_password()")?
}

async fn verify_password(
  expected_password_hash: SecretBox<String>,
  password_candidate: SecretBox<String>,
) -> ServiceResult<()> {
  task::spawn_blocking(move || {
    let parsed_hash = PasswordHash::new(expected_password_hash.expose_secret()).map_err(|err| {
      info!("Failed to parse password hash: {}", err);
      ServiceError::InvalidCredentials()
    })?;

    Argon2::default()
      .verify_password(password_candidate.expose_secret().as_bytes(), &parsed_hash)
      .map_err(|_| ServiceError::InvalidCredentials())
  })
  .await
  .context("panic in verify_password()")?
}

async fn find_user_by_username(pool: &SqlitePool, username: &str) -> ServiceResult<User> {
  sqlx::query_as::<_, User>(FIND_USER_BY_USERNAME)
    .bind(username)
    .fetch_optional(pool)
    .await?
    .ok_or(ServiceError::InvalidCredentials())
}

async fn check_user_exists(pool: &SqlitePool, email: &str) -> ServiceResult<Option<User>> {
  sqlx::query_as::<_, User>(FIND_USER_BY_EMAIL)
    .bind(email)
    .fetch_optional(pool)
    .await
    .map_err(ServiceError::DatabaseError)
}

async fn ensure_user_exists(pool: &SqlitePool, id: Uuid) -> ServiceResult<()> {
  let user_exists = sqlx::query_as::<_, User>(FIND_USER_BY_ID)
    .bind(id)
    .fetch_optional(pool)
    .await?;

  match user_exists {
    Some(_) => Ok(()),
    None => Err(ServiceError::ResourceNotFound(id.to_string())),
  }
}
