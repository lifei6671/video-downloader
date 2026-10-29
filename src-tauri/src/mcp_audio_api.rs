use super::{transcription::Source, Core};
use crate::{library, mcp_read::{AudioExcerpt, AudioInput, ReadError}, python_bridge::PythonBridge};
use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};

const SAMPLE_RATE: u64 = 16000;
const MAX_WAV_BYTES: u64 = 15 * SAMPLE_RATE * 2 + 4096;

impl Core {
    /// A request-owned decoder: dropping its transport requests cancellation and awaits reaping.
    pub async fn mcp_audio_excerpt(&self, params: Value, cancel: impl std::future::Future<Output = ()>) -> Result<Value, ReadError> {
        self.audio_excerpt(params, cancel).await.and_then(|output| Ok(serde_json::to_value(output)?)).map_err(ReadError::from_domain)
    }
    async fn audio_excerpt(&self, params: Value, cancel: impl std::future::Future<Output = ()>) -> Result<AudioExcerpt> {
        let input: AudioInput = serde_json::from_value(params).context("INVALID_INPUT")?;
        crate::mcp_read::id(&input.file_id)?;
        if input.start_ms >= input.end_ms || input.end_ms - input.start_ms > 15000 || input.end_ms > 9_007_199_254_740_991 { bail!("INVALID_INPUT"); }
        let _slot = self.audio_slots.try_acquire().map_err(|_| anyhow::anyhow!("AUDIO_BUSY"))?;
        let source = {
            let inner = self.inner.lock().unwrap();
            if inner.closing { bail!("SERVICE_UNAVAILABLE"); }
            crate::mcp_read::require_collected(&inner.db, &input.asset_id)?;
            let file = library::asset_file(&inner.db, &input.asset_id, &input.file_id)?;
            if file.missing { bail!("FILE_MISSING"); }
            if file.changed { bail!("AUDIO_SOURCE_CHANGED"); }
            if file.duration_ms.is_some_and(|duration| input.end_ms > duration) { bail!("AUDIO_RANGE_UNAVAILABLE"); }
            Source::read(&file.path)?
        };
        let stage = tempfile::Builder::new().prefix("yingqu-audio-").tempdir_in(self.runtime_directory.parent().context("AUDIO_FAILED")?).context("AUDIO_FAILED")?;
        let outcome = PythonBridge::operation_cancellable(&self.program, &self.args, &uuid::Uuid::new_v4().to_string(), "get_audio_excerpt",
            json!({"source":source.path,"staging_dir":stage.path(),"runtime_directory":self.runtime_directory,"start_ms":input.start_ms,"end_ms":input.end_ms}), |_| {}, cancel).await;
        let result = (|| -> Result<AudioExcerpt> {
            let artifact = outcome.map_err(|error| if error.to_string() == "TIMEOUT" { anyhow::anyhow!("AUDIO_TIMEOUT") } else { error })?;
            let frames = artifact["frames"].as_u64().context("AUDIO_FAILED")?;
            let start = artifact["start_sample"].as_u64().context("AUDIO_FAILED")?;
            if artifact["filename"] != "excerpt.wav" || artifact["sample_rate"] != SAMPLE_RATE
                || frames == 0 || frames > 15 * SAMPLE_RATE || start > 9_007_199_254_740_991
                || u128::from(start) * 1000 < u128::from(input.start_ms) * u128::from(SAMPLE_RATE)
                || u128::from(start + frames) * 1000 > u128::from(input.end_ms) * u128::from(SAMPLE_RATE) { bail!("AUDIO_FAILED"); }
            let path = stage.path().join("excerpt.wav");
            let metadata = std::fs::symlink_metadata(&path).context("AUDIO_FAILED")?;
            if !metadata.is_file() || metadata.len() > MAX_WAV_BYTES { bail!("AUDIO_FAILED"); }
            let body = std::fs::read(path).context("AUDIO_FAILED")?;
            validate_wav(&body, frames)?;
            {
                let inner = self.inner.lock().unwrap();
                if inner.closing { bail!("SERVICE_UNAVAILABLE"); }
                crate::mcp_read::require_collected(&inner.db, &input.asset_id)?;
                let file = library::asset_file(&inner.db, &input.asset_id, &input.file_id)?;
                if Source::read(&file.path).ok().as_ref() != Some(&source) || Source::read(&source.path).ok().as_ref() != Some(&source) { bail!("AUDIO_SOURCE_CHANGED"); }
            }
            Ok(AudioExcerpt { asset_id: input.asset_id, file_id: input.file_id, start_ms: input.start_ms, end_ms: input.end_ms,
                actual_start_ms: (start * 1000 + SAMPLE_RATE / 2) / SAMPLE_RATE,
                actual_end_ms: ((start + frames) * 1000 + SAMPLE_RATE / 2) / SAMPLE_RATE,
                format: "wav_pcm_s16le".into(), sample_rate: SAMPLE_RATE as u32, channels: 1, bits_per_sample: 16,
                frames, source_changed: false, audio_base64: Some(STANDARD.encode(body)) })
        })();
        stage.close().context("AUDIO_FAILED")?;
        result
    }
}

fn validate_wav(body: &[u8], frames: u64) -> Result<()> {
    if body.get(..4) != Some(b"RIFF") || body.get(8..12) != Some(b"WAVE") { bail!("AUDIO_FAILED"); }
    let mut offset = 12;
    let (mut format, mut samples) = (false, false);
    while offset + 8 <= body.len() {
        let size = u32::from_le_bytes(body[offset + 4..offset + 8].try_into()?) as usize;
        let end = offset + 8 + size;
        if end > body.len() { bail!("AUDIO_FAILED"); }
        let chunk = &body[offset + 8..end];
        match &body[offset..offset + 4] {
            b"fmt " => { format = size >= 16 && chunk[..4] == [1, 0, 1, 0] && chunk[4..8] == 16000u32.to_le_bytes() && chunk[14..16] == [16, 0]; }
            b"data" => { samples = size as u64 == frames * 2; }
            _ => {}
        }
        offset = end + (size % 2);
    }
    if !format || !samples { bail!("AUDIO_FAILED"); }
    Ok(())
}

#[cfg(test)]
#[path = "mcp_audio_tests.rs"]
mod tests;
