//! One local journal row per invocation of the provider's pull_changes.
use crate::desktop_core::Core;
use rusqlite::params;

pub(super) struct Run<'a> {
    core: &'a Core,
    id: Option<i64>,
}

impl<'a> Run<'a> {
    pub fn start(core: &'a Core, space: &str, trigger: &str) -> Self {
        let result = core.backup_db(|db| {
            let tx = db.unchecked_transaction()?;
            let delete_busy = super::scheduler::foreground_delete_busy(&tx, space)?;
            tx.execute(
                "INSERT INTO backup_pull_runs(space_id,trigger,foreground_delete_busy,started_at_ms)
                 VALUES(?1,?2,?3,CAST(unixepoch('subsec')*1000 AS INTEGER))",
                params![space, trigger, delete_busy],
            )?;
            let id = tx.last_insert_rowid();
            tx.execute("DELETE FROM backup_pull_runs WHERE run_id<=?1-1024", [id])?;
            tx.commit()?;
            Ok(id)
        });
        let id = result.inspect_err(|e| eprintln!("backup pull timing start: {e}")).ok();
        Self { core, id }
    }

    pub fn finish(&mut self, outcome: &str) {
        let Some(id) = self.id.take() else { return };
        if let Err(error) = self.core.backup_db(|db| {
            db.execute(
                "UPDATE backup_pull_runs SET ended_at_ms=CAST(unixepoch('subsec')*1000 AS INTEGER),
                 outcome=?1 WHERE run_id=?2",
                params![outcome, id],
            )?;
            Ok(())
        }) {
            eprintln!("backup pull timing finish: {error}");
        }
    }
}

impl Drop for Run<'_> {
    fn drop(&mut self) {
        self.finish("canceled");
    }
}
