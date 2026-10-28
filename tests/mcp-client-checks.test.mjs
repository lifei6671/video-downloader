import assert from 'node:assert/strict';
import test from 'node:test';
import { auditComparison, assertPublic, unavailable, validateModelItem } from '../scripts/mcp-client-checks.mjs';

function sample() {
  const expected = { assets: ['a', 'b'], category_id: 'cat', tag_id: 'tag', documents: [
    { asset_id: 'a', document_id: 'da', version: 2, text: '😀正确原句。' },
    { asset_id: 'b', document_id: 'db', version: 1, text: '另一条原句。' },
  ] };
  const success = (tool, args, data) => ({ tool, arguments: args, status: 'completed', error: null, result: { structuredContent: data } });
  const calls = [success('list_categories', {}, {}), success('list_tags', {}, {}), success('list_notes', {}, {})];
  const filters = { limit: 1, category_ids: ['cat'], tag_ids: ['tag'], platform: 'douyin', archived: false };
  calls.push(success('search_assets', filters, { items: [{ asset: { id: 'a' } }] }));
  calls.push(success('search_assets', { ...filters, cursor: { id: 'a', created_at: 1 } }, { items: [{ asset: { id: 'b' } }] }));
  for (const doc of expected.documents) {
    calls.push(success('get_asset', { asset_id: doc.asset_id }, { asset: { title: '样本' } }));
    calls.push(success('read_transcript', { asset_id: doc.asset_id, document_id: doc.document_id, version: doc.version }, { reference: { asset_id: doc.asset_id, document_id: doc.document_id, document_version: doc.version, kind: 'edited' }, content: { text: doc.text, offset: 0 } }));
    calls.push(success('read_note', { note_id: `n${doc.asset_id}` }, { reference: { asset_id: doc.asset_id, revision: 1 }, content: { text: '人工笔记原句' } }));
  }
  const report = { videos: expected.documents.map(doc => ({ asset_id: doc.asset_id, title: '样本', hook: '开场', visual_technique: '人工记录的方法', call_to_action: '行动',
    evidence: [{ reference: { asset_id: doc.asset_id, document_id: doc.document_id, document_version: doc.version, kind: 'edited', offset: 0, length: Array.from(doc.text).length }, quote: doc.text }],
    note_evidence: [{ note_id: `n${doc.asset_id}`, revision: 1, quote: '人工笔记' }],
  })), shared_strategy: '共同点', differences: ['差异一', '差异二'], reusable_ideas: [{ idea: '可复用', source_asset_ids: ['a', 'b'] }], limitations: ['没有视频帧和逐帧时间戳'] };
  return { expected, calls, report };
}

test('checks real calls and Unicode scalar references while leaving semantics to review', () => {
  const { expected, calls, report } = sample();
  const result = auditComparison(report, calls, expected);
  assert.equal(result.citations, 4);
  assert.equal(result.semanticQuality, 'REQUIRES_REVIEW');
});
for (const [name, mutate] of [
  ['fabricated quote', state => state.report.videos[0].evidence[0].quote = '不存在的原句'],
  ['wrong version', state => state.report.videos[0].evidence[0].reference.document_version = 1],
  ['wrong asset', state => state.report.videos[0].evidence[0].reference.asset_id = 'b'],
  ['out of range', state => state.report.videos[0].evidence[0].reference.length = 999],
  ['unfetched note', state => state.report.videos[0].note_evidence[0].note_id = 'other'],
  ['missing model calls', state => state.calls = []],
  ['failed transcript', state => state.calls.find(call => call.tool === 'read_transcript').status = 'failed'],
  ['missing transcript response', state => state.calls.find(call => call.tool === 'read_transcript').result = null],
  ['unreturned range', state => state.calls.find(call => call.tool === 'read_transcript').result.structuredContent.content.text = '😀'],
  ['wrong returned version', state => state.calls.find(call => call.tool === 'read_transcript').result.structuredContent.reference.document_version = 1],
  ['unrelated search result', state => state.calls.find(call => call.tool === 'search_assets').result.structuredContent.items[0].asset.id = 'other'],
]) test(`rejects ${name}`, () => { const state = sample(); mutate(state); assert.throws(() => auditComparison(state.report, state.calls, state.expected)); });

test('rejects public paths and synthetic private sources', () => {
  assert.throws(() => assertPublic({ path: 'local' }));
  assert.throws(() => assertPublic({ text: 'PRIVATE_WB208_SIGNED_URL' }));
});

test('only the intended MCP and ordinary message items are allowed', () => {
  validateModelItem({ type: 'agentMessage' });
  validateModelItem({ type: 'mcpToolCall', server: 'yingqu', tool: 'read_note' });
  for (const type of ['imageView', 'imageGeneration', 'subAgentActivity', 'commandExecution', 'functionCallOutput']) assert.throws(() => validateModelItem({ type }));
  assert.throws(() => validateModelItem({ type: 'mcpToolCall', server: 'other', tool: 'read_note' }));
});

test('unrelated RPC failures cannot count as disconnection or startup failure', () => {
  unavailable('call', { code: -32603, message: 'Transport closed' });
  unavailable('start', { code: -32603, message: 'required MCP server yingqu failed to start' });
  for (const kind of ['call', 'start']) assert.throws(() => unavailable(kind, { code: -32602, message: 'Invalid params' }));
});
