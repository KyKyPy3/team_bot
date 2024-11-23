CREATE TABLE IF NOT EXISTS `users` (
  `id` BLOB NOT NULL PRIMARY KEY,
  `username` TEXT NOT NULL UNIQUE COLLATE NOCASE,
  `role` TEXT NOT NULL CHECK(role IN ('user', 'admin')) DEFAULT 'user',
  `email` TEXT UNIQUE COLLATE NOCASE,
  `password` TEXT NOT NULL,
  `created_at` TIMESTAMP NOT NULL DEFAULT (DATETIME('now')),
  `updated_at` TIMESTAMP NOT NULL DEFAULT (DATETIME('now'))
);

CREATE INDEX IF NOT EXISTS idx_users_username ON users(username);
CREATE INDEX IF NOT EXISTS idx_users_email ON users(email);

CREATE TRIGGER IF NOT EXISTS trig_users_updated_at
  AFTER UPDATE ON users
  FOR EACH ROW
  BEGIN
      UPDATE users SET updated_at = DATETIME('now')
      WHERE id = NEW.id;
  END;

INSERT INTO users (id, username, role, email, password)
VALUES (
  X'01020304050607080910111213141516', -- Example UUID in hex
  'admin',
  'admin',
  'roman.efremenko@gmail.com',
  '$argon2id$v=19$m=15000,t=2,p=1$EjS1EGef1aom/gJnrCD+5w$3aa/Q2tQai4DfipYxZ3h2Gh7Kdmj3gnAad1o0197jGs'
);
