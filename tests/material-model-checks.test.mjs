import assert from 'node:assert/strict';
import test from 'node:test';
import { auditMaterialModel } from '../scripts/mcp-material-model-checks.mjs';

function sample() {
  const call = (tool, result, args = {}) => ({ tool, arguments: args, status: 'completed', error: null, result });
  const frames = [{ id: 'f1', actual_ms: 1500, image_status: 'included' }, { id: 'f2', actual_ms: 4500, image_status: 'included' }];
  const calls = [call('search_assets', {}), call('read_transcript', { structuredContent: { document: { id: 'doc-rec', version: 1, asset_id: 'asset-a', file_id: 'file-a' }, content: { text: '实际读取的原文' } } }),
    call('start_frame_sampling', {}, { asset_id: 'video', file_id: 'file' }), call('get_frame_sampling_status', {}),
    call('get_keyframes', { structuredContent: { frames: { items: frames } }, content: [{ type: 'image' }, { type: 'image' }] }), call('get_audio_excerpt', { content: [{ type: 'text', text: 'audio unsupported by this client' }] })];
  return { calls, report: { text_asset_id: 'asset-a', text_file_id: 'file-a', document_id: 'doc-rec', document_version: 1, text_analysis: '根据原文分析', media_asset_id: 'video', media_file_id: 'file', media_start_ms: 0, media_end_ms: 15000, frame_observations: frames.map(f => ({ frame_id: f.id, actual_ms: f.actual_ms })), audio_consumed: false, limitations: ['未听到实际声音'] } };
}
test('material audit separates delivered image evidence from unsupported model audio', () => {
  const { report, calls } = sample(); assert.equal(auditMaterialModel(report, calls).actualModelAudioConsumption, 'UNAVAILABLE');
});
for (const [name, mutate] of [
  ['failed transcript', s => { s.calls.find(c => c.tool === 'read_transcript').error = { code: 'unavailable' }; }],
  ['wrong file binding', s => { s.report.text_file_id = 'guessed'; }],
  ['empty text analysis', s => { s.report.text_analysis = ''; }],
  ['repeated frame', s => { s.report.frame_observations[1] = s.report.frame_observations[0]; }],
  ['unordered frames', s => { s.report.frame_observations.reverse(); }],
  ['metadata-only frame', s => { s.calls.find(c => c.tool === 'get_keyframes').result.structuredContent.frames.items[1].image_status = 'not_requested'; }],
]) test(`material audit rejects ${name}`, () => { const s = sample(); mutate(s); assert.throws(() => auditMaterialModel(s.report, s.calls)); });
