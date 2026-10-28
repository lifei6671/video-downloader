import assert from 'node:assert/strict';

export const expectedTools = ['get_asset', 'list_categories', 'list_notes', 'list_tags', 'read_note', 'read_transcript', 'search_assets'];
export const allTools = [...expectedTools, 'create_note', 'create_category', 'add_asset_categories', 'add_asset_tags', 'create_topic', 'create_idea', 'start_transcription', 'get_operation', 'cancel_operation'].sort();
export function validateModelItem(item) {
  assert(['userMessage', 'agentMessage', 'plan', 'reasoning', 'contextCompaction', 'mcpToolCall'].includes(item.type), `Unexpected item: ${item.type}`);
  if (item.type === 'mcpToolCall') { assert.equal(item.server, 'yingqu'); assert(expectedTools.includes(item.tool)); }
}
export function unavailable(kind, error) {
  const message = JSON.stringify(error);
  const matches = kind === 'call' ? /transport.{0,50}closed|disconnected|SERVICE_UNAVAILABLE|IPC_UNAVAILABLE|connection.{0,50}closed/i.test(message)
    : /IPC_UNAVAILABLE|required.{0,30}MCP.{0,100}(failed|unavailable)|MCP.{0,80}(start|initializ).{0,100}failed|MCP.{0,80}failed.{0,80}(start|initializ)/i.test(message);
  assert(matches, `Expected MCP ${kind} unavailability, not an unrelated RPC error: ${message}`);
  return { status: 'PASSED', code: error.code ?? null, message: error.message ?? message };
}
const referenceProperties = { asset_id: { type: 'string' }, document_id: { type: 'string' }, document_version: { type: 'integer' }, kind: { type: 'string' }, offset: { type: 'integer' }, length: { type: 'integer' } };
const object = properties => ({ type: 'object', additionalProperties: false, required: Object.keys(properties), properties });
const evidence = object({ reference: object(referenceProperties), quote: { type: 'string' } });
export const comparisonSchema = object({
  videos: { type: 'array', minItems: 2, maxItems: 2, items: object({
    asset_id: { type: 'string' }, title: { type: 'string' }, hook: { type: 'string' }, visual_technique: { type: 'string' }, call_to_action: { type: 'string' },
    evidence: { type: 'array', minItems: 1, items: evidence },
    note_evidence: { type: 'array', minItems: 1, items: object({ note_id: { type: 'string' }, revision: { type: 'integer' }, quote: { type: 'string' } }) },
  }) },
  shared_strategy: { type: 'string' }, differences: { type: 'array', minItems: 2, items: { type: 'string' } },
  reusable_ideas: { type: 'array', minItems: 1, items: object({ idea: { type: 'string' }, source_asset_ids: { type: 'array', items: { type: 'string' } } }) },
  limitations: { type: 'array', minItems: 1, items: { type: 'string' } },
});
export const comparisonPrompt = `这是一次自媒体知识库的合成验收，只允许使用 yingqu MCP 的只读工具，不运行命令、不读文件、不用网页、其他服务或子代理。
请比较“创作学习”分类（包含子分类）、“WB208验收”标签、douyin 平台下未归档的两条拖延学习视频。
先用 list_categories 和 list_tags 发现 ID，再用 search_assets 同时传分类、标签、平台、archived=false，limit=1，沿 next_cursor 分页找齐结果。不要猜 ID。
对每条结果调用 get_asset，读取最新整理稿；read_transcript 明确传该 document_id 和 version。用 list_notes 和 read_note 读取人工笔记。不要用旧整理稿替代最新版本。
比较开场钩子、文案中描述的画面方法、行动引导、共同策略、至少两点差异和可复用思路。输出中文 JSON，按给定 schema。
每条视频给出文案原句 quote 及工具返回的完整 reference（asset_id、document_id、document_version、kind、offset、length），quote 必须落在该范围内；还要给人工笔记原句及 note_id/revision。不要编造引用或缩写原句。
明确说明工具没有提供视频帧或逐帧时间戳，画面方法依据文案和笔记，不能声称看过原视频。limitations 只写资料和分析的局限，不讨论对话标题、工具权限或操作过程。不要写回资料。`;

export function assertPublic(value) {
  const forbidden = new Set(['path', 'source', 'source_json', 'canonical_url', 'cookie', 'proxy', 'credential_ref', 'user_agent']);
  function visit(item) {
    if (!item || typeof item !== 'object') return;
    for (const [key, child] of Object.entries(item)) { assert(!forbidden.has(key), `Private field: ${key}`); visit(child); }
  }
  visit(value);
  assert(!JSON.stringify(value).includes('PRIVATE_WB208_'), 'Private synthetic path/source leaked');
}

export function content(result) {
  assert(!result.isError, JSON.stringify(result));
  const value = result.structuredContent ?? JSON.parse(result.content.find(item => item.type === 'text').text);
  assertPublic(value);
  return value;
}

export async function verifyReads(call) {
  const categories = content(await call('list_categories', {}));
  const tags = content(await call('list_tags', {}));
  assert(JSON.stringify(categories).includes('wb208-learning'));
  assert(JSON.stringify(tags).includes('wb208-tag'));
  const filter = { query: '任务', category_ids: ['wb208-learning'], tag_ids: ['wb208-tag'], platform: 'douyin', archived: false, limit: 1 };
  const first = content(await call('search_assets', filter));
  assert.equal(first.items.length, 1); assert(first.next_cursor);
  const second = content(await call('search_assets', { ...filter, cursor: first.next_cursor }));
  assert.deepEqual([...first.items, ...second.items].map(item => item.asset.id).sort(), ['wb208-a', 'wb208-b']);
  assert.equal(second.next_cursor, null);
  const asset = content(await call('get_asset', { asset_id: 'wb208-a' }));
  assert(JSON.stringify(asset).includes('wb208-doc-a2'));
  const document = content(await call('read_transcript', { asset_id: 'wb208-a', document_id: 'wb208-doc-a2', version: 2, offset: 0, length: 16 }));
  assert.equal(document.reference.document_version, 2); assert.equal(Array.from(document.content.text).length, 16);
  const notes = content(await call('list_notes', { asset_id: 'wb208-a', limit: 1 }));
  assert.equal(notes.notes.length, 1);
  const note = content(await call('read_note', { asset_id: 'wb208-a', note_id: 'wb208-a-note', revision: 1 }));
  assert.equal(note.reference.revision, 1);
  const empty = content(await call('search_assets', { query: '没有这条合成资料', limit: 1 }));
  assert.deepEqual(empty.items, []);
  const errors = [];
  for (const [tool, input, expected] of [
    ['get_asset', { asset_id: 'unknown-wb208' }, 'ASSET_NOT_FOUND'],
    ['search_assets', { cursor: { created_at: 999, id: 'not-a-real-cursor' }, limit: 1 }, 'INVALID_CURSOR'],
    ['read_transcript', { asset_id: 'wb208-a', document_id: 'wb208-doc-a2', version: 2, offset: 99999 }, 'OUT_OF_RANGE'],
    ['read_transcript', { asset_id: 'wb208-a', length: 16001 }, 'INVALID_INPUT'],
    ['read_transcript', { asset_id: 'wb208-a', document_id: 'wb208-doc-a2', version: 1 }, 'VERSION_CONFLICT'],
  ]) {
    let result;
    try { result = await call(tool, input); assert(result.isError, `Expected ${expected}`); }
    catch (error) { if (!error.rpcError) throw error; result = error.rpcError; }
    assert(JSON.stringify(result).includes(expected), `Expected ${expected}: ${JSON.stringify(result)}`);
    assertPublic(result); errors.push(expected);
  }
  return { filtersPagination: 'PASSED', sevenReads: 'PASSED', unicode: 'PASSED', emptyAndBoundaries: errors };
}

export function auditComparison(report, calls, expected) {
  const successful = calls.filter(call => call.status === 'completed' && !call.error && call.result && !call.result.isError);
  assert.deepEqual(report.videos.map(video => video.asset_id).sort(), [...expected.assets].sort());
  assert(expectedTools.every(tool => successful.some(call => call.tool === tool)), 'The model must successfully use all seven tools');
  const searches = successful.filter(call => call.tool === 'search_assets');
  assert(searches.length >= 2 && searches.every(call => call.arguments.limit === 1 && call.arguments.category_ids?.includes(expected.category_id) && call.arguments.tag_ids?.includes(expected.tag_id) && call.arguments.platform === 'douyin' && call.arguments.archived === false), 'Missing combined model filtering');
  assert(searches.some(call => call.arguments.cursor), 'Model skipped requested search pagination');
  const found = searches.flatMap(call => content(call.result).items.map(item => item.asset.id));
  assert.deepEqual([...new Set(found)].sort(), [...expected.assets].sort(), 'Search returned an unrelated or archived asset');
  let citations = 0;
  for (const video of report.videos) {
    const source = expected.documents.find(document => document.asset_id === video.asset_id);
    assert(source && video.hook && video.visual_technique && video.call_to_action);
    const metadata = successful.find(call => call.tool === 'get_asset' && call.arguments.asset_id === video.asset_id);
    assert(metadata, 'Asset metadata was never fetched');
    assert.equal(content(metadata.result).asset.title, video.title);
    assert(video.evidence.length > 0 && video.note_evidence.length > 0);
    for (const item of video.evidence) {
      const ref = item.reference;
      assert.equal(ref.asset_id, video.asset_id); assert.equal(ref.document_id, source.document_id); assert.equal(ref.document_version, source.version); assert.equal(ref.kind, 'edited');
      assert(Number.isSafeInteger(ref.offset) && ref.offset >= 0 && Number.isSafeInteger(ref.length) && ref.length > 0);
      const text = Array.from(source.text);
      assert(ref.offset + ref.length <= text.length);
      assert(item.quote.length > 0 && text.slice(ref.offset, ref.offset + ref.length).join('').includes(item.quote), 'Quotation must match its scalar range');
      const fetched = successful.filter(call => call.tool === 'read_transcript' && call.arguments.asset_id === ref.asset_id && call.arguments.document_id === ref.document_id && call.arguments.version === ref.document_version).map(call => content(call.result));
      assert(fetched.some(document => document.reference.asset_id === ref.asset_id && document.reference.document_id === ref.document_id && document.reference.document_version === ref.document_version && document.reference.kind === ref.kind
        && document.content.offset <= ref.offset && ref.offset + ref.length <= document.content.offset + Array.from(document.content.text).length
        && Array.from(document.content.text).slice(ref.offset - document.content.offset, ref.offset - document.content.offset + ref.length).join('').includes(item.quote)), 'Quote range was never successfully returned to the model');
      citations++;
    }
    for (const item of video.note_evidence) {
      const fetched = successful.find(call => call.tool === 'read_note' && call.arguments.note_id === item.note_id);
      assert(fetched, 'Note was never fetched by the model');
      const note = content(fetched.result);
      assert.equal(note.reference.asset_id, video.asset_id); assert.equal(note.reference.revision, item.revision);
      assert(item.quote.length > 0 && note.content.text.includes(item.quote), 'Note quote mismatch');
      citations++;
    }
  }
  assert(report.shared_strategy && report.differences.length >= 2 && report.reusable_ideas.length > 0);
  assert(report.reusable_ideas.every(idea => idea.source_asset_ids.length > 0 && idea.source_asset_ids.every(id => expected.assets.includes(id))));
  assert(report.limitations.some(text => /时间戳|视频帧|未观看|没有观看|未直接观看/.test(text)));
  assert(!JSON.stringify(report).includes('过时整理稿'));
  assertPublic(report);
  return { selectedAssets: 'PASSED', fetchedVersions: 'PASSED', verbatimQuotesAndRanges: 'PASSED', citations, actualModelTools: [...new Set(successful.map(call => call.tool))].sort(), semanticQuality: 'REQUIRES_REVIEW' };
}
