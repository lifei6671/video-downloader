//! Wait for task cleanup before admitting an explicit user action.
//! Only precondition failures may be repeated; file/remote deletion is invoked once.
use super::{Core, Status, Task};
use anyhow::{bail, Context, Result};
use std::{sync::Arc, time::Duration};
use tokio::sync::watch;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Retry,
    Remove,
    Delete,
}

type Outcome = std::result::Result<Option<Task>, String>;

pub(super) struct PendingAction {
    action: Action,
    revision: u64,
    cancel: watch::Sender<bool>,
    result: watch::Sender<Option<Outcome>>,
}

struct Registration<'a> {
    core: &'a Core,
    id: &'a str,
    result: watch::Sender<Option<Outcome>>,
}

impl Drop for Registration<'_> {
    fn drop(&mut self) {
        // Also release followers if the IPC future itself is dropped.
        self.result.send_if_modified(|result| {
            if result.is_some() {
                return false;
            }
            *result = Some(Err("CANCELED".into()));
            true
        });
        self.core.task_actions.lock().unwrap().remove(self.id);
    }
}

impl Core {
    pub(super) fn cancel_waiting_task_action(&self, id: &str) {
        if let Some(action) = self.task_actions.lock().unwrap().get(id) {
            action.cancel.send_replace(true);
        }
    }

    pub(super) fn close_task_action_admission(&self) {
        // Admission and shutdown use the same lock order. There must be no gap
        // between closing admission and stopping the already registered waiters.
        let pending = self.task_actions.lock().unwrap();
        self.inner.lock().unwrap().closing = true;
        for action in pending.values() {
            action.cancel.send_replace(true);
        }
    }

    pub async fn task_action(self: &Arc<Self>, id: &str, action: Action) -> Result<Option<Task>> {
        let (mut cancel, result, follower) = {
            let mut pending = self.task_actions.lock().unwrap();
            let inner = self.inner.lock().unwrap();
            if inner.closing {
                bail!("CANCELED");
            }
            let task = inner
                .tasks
                .iter()
                .find(|task| task.id == id)
                .context("TASK_NOT_FOUND")?;
            if let Some(existing) = pending.get(id) {
                if existing.action != action || existing.revision != task.revision {
                    bail!("INVALID_STATE");
                }
                (existing.cancel.subscribe(), existing.result.clone(), true)
            } else {
                if !matches!(
                    task.status,
                    Status::Completed | Status::Failed | Status::Canceled
                ) || (action == Action::Retry && task.status == Status::Completed)
                {
                    bail!("INVALID_STATE");
                }
                let (stop, cancel) = watch::channel(false);
                let (result, _) = watch::channel(None);
                pending.insert(
                    id.to_owned(),
                    PendingAction {
                        action,
                        revision: task.revision,
                        cancel: stop,
                        result: result.clone(),
                    },
                );
                (cancel, result, false)
            }
        };
        if follower {
            let mut reply = result.subscribe();
            reply
                .wait_for(|value| value.is_some())
                .await
                .context("CANCELED")?;
            return reply
                .borrow()
                .clone()
                .context("CANCELED")?
                .map_err(anyhow::Error::msg);
        }
        let registration = Registration {
            core: self,
            id,
            result,
        };
        let outcome = self.execute_task_action(id, action, &mut cancel).await;
        registration
            .result
            .send_replace(Some(outcome.as_ref().cloned().map_err(ToString::to_string)));
        outcome
    }

    async fn execute_task_action(
        self: &Arc<Self>,
        id: &str,
        action: Action,
        cancel: &mut watch::Receiver<bool>,
    ) -> Result<Option<Task>> {
        loop {
            let admitted = {
                // Serialize cancellation and admission. Never hold either mutex over an await.
                let pending = self.task_actions.lock().unwrap();
                let entry = pending.get(id).context("CANCELED")?;
                if *cancel.borrow() {
                    bail!("CANCELED");
                }
                let inner = self.inner.lock().unwrap();
                if inner.closing {
                    bail!("CANCELED");
                }
                let task = inner
                    .tasks
                    .iter()
                    .find(|task| task.id == id)
                    .context("TASK_NOT_FOUND")?;
                if task.revision != entry.revision {
                    bail!("TASK_ACTION_STALE");
                }
                let busy = inner.active.contains_key(id)
                    || inner.downloads.contains(id)
                    || (action == Action::Retry
                        && inner
                            .tasks
                            .iter()
                            .any(|task| task.status == Status::Parsing));
                drop(inner);
                if busy {
                    false
                } else if action == Action::Retry {
                    match self.retry(id) {
                        Ok(task) => return Ok(Some(task)),
                        Err(error) if error.to_string() == "TASK_BUSY" => false,
                        Err(error) => return Err(error),
                    }
                } else {
                    true
                }
            };
            if admitted {
                break;
            }
            tokio::select! {
                _ = cancel.changed() => bail!("CANCELED"),
                _ = tokio::time::sleep(Duration::from_millis(250)) => {}
            }
        }
        // No replay of an async lifecycle operation: it may already have changed a
        // file or committed a remote delete intent. Its existing journal owns recovery.
        tokio::select! {
            biased;
            _ = cancel.changed() => bail!("CANCELED"),
            result = async {
                match action {
                    Action::Remove => self.remove_after_transcription(id, false).await,
                    Action::Delete => crate::backup::lifecycle::delete_task(self, id).await,
                    Action::Retry => unreachable!(),
                }
            } => result?,
        }
        Ok(None)
    }
}
