use sqlx::SqlitePool;
use uuid::Uuid;

use crate::ServiceResult;
use platform_bot_entity::user::User;

const LIST_USERS_QUERY: &str = "SELECT * FROM users ORDER BY id LIMIT ? OFFSET ?";
const FIND_USER_BY_ID_QUERY: &str = "SELECT * FROM users WHERE id = ?1";
const COUNT_USERS_QUERY: &str = "SELECT COUNT(*) FROM users";

/// Lists users with pagination
///
/// # Arguments
/// * `pool` - Database connection pool
/// * `page` - Page number (1-based)
/// * `limit` - Number of items per page
///
/// # Returns
/// A tuple containing the users and total number of pages
pub async fn list(pool: &SqlitePool, page: i64, limit: i64) -> ServiceResult<(Vec<User>, i64)> {
  let (total_count, users) = tokio::try_join!(get_total_count(pool), fetch_paginated_users(pool, page, limit))?;

  let total_pages = calculate_total_pages(total_count, limit);
  Ok((users, total_pages))
}

/// Finds a user by their ID
///
/// # Arguments
/// * `pool` - Database connection pool
/// * `id` - User UUID to search for
///
/// # Returns
/// Optional User if found
pub async fn find_by_id(pool: &SqlitePool, id: Uuid) -> ServiceResult<Option<User>> {
  sqlx::query_as::<_, User>(FIND_USER_BY_ID_QUERY)
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

async fn get_total_count(pool: &SqlitePool) -> ServiceResult<i64> {
  let (count,): (i64,) = sqlx::query_as(COUNT_USERS_QUERY).fetch_one(pool).await?;
  Ok(count)
}

fn calculate_total_pages(total_count: i64, limit: i64) -> i64 {
  (total_count as f64 / limit as f64).ceil() as i64
}

async fn fetch_paginated_users(pool: &SqlitePool, page: i64, limit: i64) -> ServiceResult<Vec<User>> {
  let offset = (page - 1) * limit;

  sqlx::query_as::<_, User>(LIST_USERS_QUERY)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_calculate_total_pages() {
    assert_eq!(calculate_total_pages(10, 3), 4);
    assert_eq!(calculate_total_pages(10, 5), 2);
    assert_eq!(calculate_total_pages(0, 5), 0);
  }
}
