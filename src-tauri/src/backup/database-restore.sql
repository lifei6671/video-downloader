-- Keep portable history, cached metadata and cloud membership. Paths and
-- operation evidence refer to files, credentials and processes on the old host.
UPDATE download_tasks SET snapshot=(SELECT original FROM download_replacements WHERE task_id=download_tasks.id)
 WHERE EXISTS(SELECT 1 FROM download_replacements WHERE task_id=download_tasks.id);
UPDATE download_tasks SET snapshot=json_set(snapshot,'$.path',NULL,'$.cover_path',NULL)
 WHERE json_extract(snapshot,'$.path') IS NOT NULL OR json_extract(snapshot,'$.cover_path') IS NOT NULL;
UPDATE download_tasks SET snapshot=json_set(snapshot,'$.status','failed','$.error','DATABASE_RESTORE_INTERRUPTED',
 '$.canceling',json('false'),'$.merging',json('false'),'$.speed_bps',NULL,'$.eta_seconds',NULL,'$.queue_order',NULL)
 WHERE json_extract(snapshot,'$.status') IN ('queued','downloading','parsing')
    OR json_extract(snapshot,'$.error') IN ('INTERRUPTED','DATABASE_ERROR','REDOWNLOAD_RESTORE_FAILED');
DELETE FROM commit_intents;
DELETE FROM task_staging;
DELETE FROM download_replacements;
UPDATE asset_local_files SET path=NULL,source_json=NULL,verified_at=NULL,local_state='missing';
UPDATE asset_transcripts SET source_json=NULL WHERE source_json IS NOT NULL;
UPDATE platform_settings SET credential_ref=NULL;
DELETE FROM telegram_account_bindings;
DELETE FROM telegram_media_sends;
DELETE FROM telegram_extra_sends;
DELETE FROM telegram_transfer_cooldowns;
DELETE FROM backup_probe_cleanup;
DELETE FROM backup_credential_cleanup;
DELETE FROM backup_operations;
DELETE FROM backup_upload_proofs;
DELETE FROM backup_import_receipts;
DELETE FROM asset_deletion_hidden_history;
UPDATE backup_record_retention SET enabled=0,last_error=NULL,next_check_at=0;
UPDATE backup_space_assets SET pending_json='{}';
UPDATE asset_remote_objects SET remote_state=CASE
 WHEN EXISTS(SELECT 1 FROM backup_space_assets m WHERE m.space_id=asset_remote_objects.space_id AND m.local_asset_id=asset_remote_objects.asset_id AND m.remote_deleted=1) THEN 'deleted'
 WHEN EXISTS(SELECT 1 FROM asset_remote_records r WHERE r.space_id=asset_remote_objects.space_id AND r.asset_id=asset_remote_objects.asset_id) THEN 'backed_up'
 ELSE 'not_backed_up' END WHERE remote_state<>'deleted';
UPDATE backup_spaces SET credential_ref=NULL,enabled=0;
DELETE FROM backup_current_space;
DELETE FROM backup_sync_preference;
DELETE FROM backup_databases;
