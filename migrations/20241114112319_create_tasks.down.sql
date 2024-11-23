DROP TRIGGER IF EXISTS trig_tasks_updated_at;
DROP INDEX IF EXISTS idx_tasks_project_id;
DROP INDEX IF EXISTS idx_tasks_external_id;
DROP INDEX IF EXISTS idx_tasks_status;
DROP INDEX IF EXISTS idx_tasks_start_at;
DROP TABLE IF EXISTS `tasks`;
