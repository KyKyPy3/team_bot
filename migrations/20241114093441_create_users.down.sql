-- Drop triggers first
DROP TRIGGER IF EXISTS trig_users_updated_at;

-- Drop indexes
DROP INDEX IF EXISTS idx_users_username;
DROP INDEX IF EXISTS idx_users_email;

-- Drop the table
DROP TABLE IF EXISTS `users`;
