use super::{transcription::Snapshot, Core};
use crate::mcp_operations::{Operation, OperationError, OperationInput, StartInput, State};
use anyhow::{bail, Context, Result};
use rusqlite::OptionalExtension;
use serde_json::Value;
use std::sync::Arc;

fn id(value: &str) -> Result<()> {
    if value.is_empty() || value.len() > 128 {
        bail!("INVALID_INPUT");
    }
    Ok(())
}
fn public(snapshot: Snapshot, cancel_requested: bool) -> Result<Operation> {
    let state = match snapshot.state.as_str() {
        "running" => State::Running,
        "completed" => State::Completed,
        "no_speech" => State::NoSpeech,
        "failed" => State::Failed,
        "canceled" => State::Canceled,
        "interrupted" => State::Interrupted,
        _ => bail!("CORE_OPERATION_FAILED"),
    };
    let error_code = snapshot.error_code.as_deref().map(|error| {
        match error {
            "CANCELED" => "CANCELED",
            "DATABASE_ERROR" => "DATABASE_ERROR",
            "TRANSCRIPTION_INTERRUPTED" => "TRANSCRIPTION_INTERRUPTED",
            "TRANSCRIPTION_TIMEOUT" => "TRANSCRIPTION_TIMEOUT",
            "TRANSCRIPTION_SOURCE_CHANGED" => "TRANSCRIPTION_SOURCE_CHANGED",
            "INVALID_ENGINE_RESULT" => "INVALID_ENGINE_RESULT",
            "ENGINE_PROTOCOL_ERROR" => "ENGINE_PROTOCOL_ERROR",
            other => crate::python_bridge::error_code(&serde_json::json!({"error":{"code":other}})),
        }
        .to_owned()
    });
    Ok(Operation {
        operation_id: snapshot.operation_id,
        asset_id: snapshot.asset_id.context("CORE_OPERATION_FAILED")?,
        file_id: snapshot.file_id.context("CORE_OPERATION_FAILED")?,
        cancel_requested: state == State::Running && cancel_requested,
        state,
        phase: snapshot.phase.filter(|p| {
            matches!(
                p.as_str(),
                "extracting_audio" | "segmenting" | "loading_model" | "recognizing"
            )
        }),
        backend: snapshot
            .backend
            .filter(|b| matches!(b.as_str(), "cpu" | "cuda")),
        completed_segments: snapshot.completed_segments,
        total_segments: snapshot.total_segments,
        error_code,
        has_result: snapshot.has_result,
        sequence: snapshot.sequence,
        updated_at: snapshot.updated_at,
    })
}
impl Core {
    /// Additive, bounded control calls. The worker belongs to Core, not the IPC session.
    pub fn mcp_operation_tool(
        self: &Arc<Self>,
        method: &str,
        params: Value,
    ) -> Result<Value, OperationError> {
        if matches!(method, "mcp_start_frame_sampling" | "mcp_get_frame_sampling_status" | "mcp_cancel_frame_sampling") {
            return self.mcp_frame_sampling(method, params).map_err(OperationError::from_domain);
        }
        let result = (|| -> Result<Operation> {
            if method == "mcp_start_transcription" {
                let input: StartInput = serde_json::from_value(params).context("INVALID_INPUT")?;
                id(&input.asset_id)?;
                id(&input.file_id)?;
                let snapshot = self.start_mcp_asset_transcription(
                    &input.asset_id,
                    &input.file_id,
                    input.force,
                )?;
                let inner = self.inner.lock().unwrap();
                let pending = inner.transcription.as_ref().is_some_and(|a| {
                    a.snapshot.operation_id == snapshot.operation_id && *a.cancel.borrow()
                });
                return public(snapshot, pending);
            }
            if !matches!(method, "mcp_get_operation" | "mcp_cancel_operation") {
                bail!("INVALID_INPUT");
            }
            let input: OperationInput = serde_json::from_value(params).context("INVALID_INPUT")?;
            id(&input.operation_id)?;
            let inner = self.inner.lock().unwrap();
            if inner.closing {
                bail!("SERVICE_UNAVAILABLE");
            }
            if method == "mcp_cancel_operation" && !inner.mcp_write_enabled {
                bail!("WRITE_DISABLED");
            }
            if let Some(active) = inner
                .transcription
                .as_ref()
                .filter(|a| a.snapshot.operation_id == input.operation_id)
            {
                crate::mcp_read::require_collected(&inner.db, active.snapshot.asset_id.as_deref().context("ASSET_NOT_FOUND")?)?;
                if method == "mcp_cancel_operation" {
                    let _ = active.cancel.send(true);
                }
                return public(active.snapshot.summary(), *active.cancel.borrow());
            }
            if let Some(snapshot) = inner
                .transcription_unsaved
                .values()
                .find(|s| s.operation_id == input.operation_id)
            {
                crate::mcp_read::require_collected(&inner.db, snapshot.asset_id.as_deref().context("ASSET_NOT_FOUND")?)?;
                return public(snapshot.summary(), false);
            }
            let stored: String = inner.db.query_row("SELECT snapshot FROM asset_transcriptions WHERE json_extract(snapshot,'$.operation_id')=?1",[&input.operation_id],|r|r.get(0)).optional()?.context("OPERATION_NOT_FOUND")?;
            let snapshot: Snapshot = serde_json::from_str(&stored)?;
            crate::mcp_read::require_collected(&inner.db, snapshot.asset_id.as_deref().context("ASSET_NOT_FOUND")?)?;
            public(snapshot, false)
        })();
        result
            .and_then(|value| Ok(serde_json::to_value(value)?))
            .map_err(OperationError::from_domain)
    }
    fn mcp_frame_sampling(self: &Arc<Self>, method: &str, params: Value) -> Result<Value> {
        let snapshot = if method == "mcp_start_frame_sampling" {
            let input: crate::mcp_operations::FrameSamplingInput = serde_json::from_value(params).context("INVALID_INPUT")?;
            id(&input.asset_id)?; id(&input.file_id)?;
            self.start_mcp_keyframes(&input.asset_id, super::keyframes::KeyframeRequest::Range {
                file_id: input.file_id, start_ms: input.start_ms, end_ms: input.end_ms, count: input.count,
            })?
        } else {
            let input: OperationInput = serde_json::from_value(params).context("INVALID_INPUT")?;
            id(&input.operation_id)?;
            let mut inner = self.inner.lock().unwrap();
            if inner.closing { bail!("SERVICE_UNAVAILABLE"); }
            if method == "mcp_cancel_frame_sampling" && !inner.mcp_write_enabled { bail!("WRITE_DISABLED"); }
            let asset_id = inner.keyframe_run.as_ref().filter(|run| run.snapshot.id == input.operation_id)
                .context("KEYFRAME_OPERATION_NOT_FOUND")?.snapshot.asset_id.clone();
            crate::mcp_read::require_collected(&inner.db, &asset_id)?;
            let run = inner.keyframe_run.as_mut().context("KEYFRAME_OPERATION_NOT_FOUND")?;
            if method == "mcp_cancel_frame_sampling" && run.snapshot.state == "running" {
                run.snapshot.canceling = true; run.snapshot.sequence += 1;
                let _ = run.cancel.send(true);
            }
            run.snapshot.clone()
        };
        Ok(serde_json::to_value(crate::mcp_operations::FrameSamplingStatus {
            operation_id: snapshot.id, asset_id: snapshot.asset_id, file_id: snapshot.file_id,
            state: snapshot.state.into(), start_ms: snapshot.start_ms, end_ms: snapshot.end_ms,
            requested_ms: snapshot.requested_ms, completed: snapshot.completed, total: snapshot.total,
            sequence: snapshot.sequence, canceling: snapshot.canceling, error: snapshot.error, frame_ids: snapshot.frame_ids,
        })?)
    }
}

#[cfg(test)]
#[path = "mcp_operations_tests.rs"]
mod tests;
