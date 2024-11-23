CREATE TABLE IF NOT EXISTS `tasks` (
  `id` BLOB NOT NULL PRIMARY KEY,
  `type` TEXT NOT NULL CHECK(type IN ('notify', 'gerrit', 'teamcity')),
  `status` TEXT NOT NULL DEFAULT 'new' CHECK(status IN ('new', 'in_progress', 'failed', 'finished', 'retried')),
  `project_id` BLOB NOT NULL,
  `retries` INTEGER NOT NULL DEFAULT 0,
  `name` TEXT NOT NULL,
  `external_id` TEXT UNIQUE,
  `external_modified_at` TIMESTAMP,
  `schedule` TEXT,
  `start_at` INTEGER NOT NULL,
  `options` TEXT NOT NULL DEFAULT '{}' CHECK(json_valid(options)),
  `created_at` TIMESTAMP NOT NULL DEFAULT (DATETIME('now')),
  `updated_at` TIMESTAMP NOT NULL DEFAULT (DATETIME('now')),

  FOREIGN KEY(project_id) REFERENCES projects(id) ON DELETE CASCADE
);

-- Create indexes for better query performance
CREATE INDEX IF NOT EXISTS idx_tasks_project_id ON tasks(project_id);
CREATE INDEX IF NOT EXISTS idx_tasks_external_id ON tasks(external_id);
CREATE INDEX IF NOT EXISTS idx_tasks_status ON tasks(status);
CREATE INDEX IF NOT EXISTS idx_tasks_start_at ON tasks(start_at);

-- Create trigger to automatically update updated_at
CREATE TRIGGER IF NOT EXISTS trig_tasks_updated_at
  AFTER UPDATE ON tasks
  FOR EACH ROW
  BEGIN
      UPDATE tasks SET updated_at = DATETIME('now')
      WHERE id = NEW.id;
  END;
