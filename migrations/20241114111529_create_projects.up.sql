CREATE TABLE IF NOT EXISTS `projects` (
  `id` BLOB NOT NULL PRIMARY KEY,
  `name` TEXT NOT NULL,
  `code` TEXT NOT NULL UNIQUE COLLATE NOCASE,
  `sync_with_exchange` INTEGER NOT NULL DEFAULT 1 CHECK(sync_with_exchange IN (0, 1)),
  `options` TEXT DEFAULT '{}' CHECK(json_valid(options)),
  `owner_id` BLOB NOT NULL,
  `created_at` TIMESTAMP NOT NULL DEFAULT (DATETIME('now')),
  `updated_at` TIMESTAMP NOT NULL DEFAULT (DATETIME('now')),

  FOREIGN KEY(owner_id) REFERENCES users(id) ON DELETE CASCADE
);

-- Create indexes for better query performance
CREATE INDEX IF NOT EXISTS idx_projects_owner_id ON projects(owner_id);
CREATE INDEX IF NOT EXISTS idx_projects_code ON projects(code);

-- Create trigger to automatically update updated_at
CREATE TRIGGER IF NOT EXISTS trig_projects_updated_at
  AFTER UPDATE ON projects
  FOR EACH ROW
  BEGIN
      UPDATE projects SET updated_at = DATETIME('now')
      WHERE id = NEW.id;
  END;

-- Insert initial project
INSERT INTO projects (id, name, code, owner_id)
VALUES (
  X'CE15D416FDAB45798B0DE7C93EC53DBB',
  'platform',
  'ppf',
  X'01020304050607080910111213141516'
);
