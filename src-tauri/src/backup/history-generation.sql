CREATE TABLE IF NOT EXISTS backup_history_generation(id INTEGER PRIMARY KEY CHECK(id=1),generation INTEGER NOT NULL);
INSERT OR IGNORE INTO backup_history_generation VALUES(1,0);
CREATE TRIGGER IF NOT EXISTS backup_history_insert AFTER INSERT ON download_tasks BEGIN
 UPDATE backup_history_generation SET generation=generation+1 WHERE id=1;
END;
CREATE TRIGGER IF NOT EXISTS backup_history_delete AFTER DELETE ON download_tasks BEGIN
 UPDATE backup_history_generation SET generation=generation+1 WHERE id=1;
END;
CREATE TRIGGER IF NOT EXISTS backup_history_update AFTER UPDATE OF snapshot ON download_tasks
WHEN json_extract(OLD.snapshot,'$.status') IS NOT json_extract(NEW.snapshot,'$.status')
 OR json_extract(OLD.snapshot,'$.path') IS NOT json_extract(NEW.snapshot,'$.path')
 OR json_extract(OLD.snapshot,'$.asset_id') IS NOT json_extract(NEW.snapshot,'$.asset_id')
BEGIN UPDATE backup_history_generation SET generation=generation+1 WHERE id=1; END;
CREATE TRIGGER IF NOT EXISTS backup_history_detach AFTER DELETE ON backup_space_assets BEGIN
 UPDATE backup_history_generation SET generation=generation+1 WHERE id=1;
END;
