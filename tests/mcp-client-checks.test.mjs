import assert from 'node:assert/strict';
import test from 'node:test';
import { auditComparison, assertPublic, unavailable, validateModelItem } from '../scripts/mcp-client-checks.mjs';
import { auditMediaModel } from '../scripts/mcp-media-checks.mjs';
import { auditP2 } from '../scripts/mcp-p2-checks.mjs';

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

test('media audit rejects guessed metadata, missing image delivery and unobserved video claims', () => {
  const frames = [
    {id:'media-frame-0',color:'red',pts:500,time_base_num:1,time_base_den:1000,sample_start_ms:100,sample_end_ms:900,batch_id:'media-batch-0'},
    {id:'media-frame-1',color:'green',pts:750,time_base_num:1,time_base_den:1000,sample_start_ms:750,sample_end_ms:null,batch_id:'media-batch-1'},
  ];
  const report = {frames,video_observed:false,audio_observed:false,limitations:'仅观察静态采样，没有连续视频或声音'};
  const calls = [{tool:'get_clip',status:'completed'}, {tool:'get_keyframes',status:'completed',result:{content:[{type:'image'},{type:'image'}]}}];
  assert.equal(auditMediaModel(report,calls).status,'PASSED');
  for (const mutate of [r => r.frames[0].color='green', r => r.frames[0].pts=501, r => r.frames[1].sample_end_ms=900, r => r.video_observed=true, r => r.audio_observed=true]) {
    const invalid = structuredClone(report); mutate(invalid); assert.throws(() => auditMediaModel(invalid,calls));
  }
  assert.throws(() => auditMediaModel(report,[]));
  const missing = structuredClone(calls); missing[1].result.content=[]; assert.throws(() => auditMediaModel(report,missing));
  validateModelItem({type:'mcpToolCall',server:'yingqu',tool:'get_keyframes'},['get_keyframes']);
  assert.throws(() => validateModelItem({type:'mcpToolCall',server:'yingqu',tool:'create_note'},['get_keyframes']));
});

test('P2 audit requires delivered samples, exact source versions and observed clip references', () => {
  const expected={assets:['a','b'].map(id=>({id,frames:[{id:`f-${id}`,clip_id:`c-${id}`,pts:100,time_base_num:1,time_base_den:1000,sample_start_ms:0,sample_end_ms:200,batch_id:`batch-${id}`,file_id:`file-${id}`}],document:id==='a'?{id:'doc-a',version:1,text:'真实原句😀'}:null}))};
  const ok=(tool,args,data,images=[])=>({tool,arguments:args,status:'completed',result:{structuredContent:data,content:images}});
  const calls=expected.assets.flatMap(a=>[
    ok('get_asset',{asset_id:a.id},{}),
    ok('get_clip',{asset_id:a.id},{clips:{items:[{id:`c-${a.id}`}]}}),
    ok('get_keyframes',{asset_id:a.id},{frames:{items:a.frames.map(f=>({...f,image_status:'included'}))}},[{type:'image'}]),
  ]);
  calls.push(ok('read_transcript',{asset_id:'a',document_id:'doc-a'},{reference:{asset_id:'a',document_id:'doc-a',document_version:1},content:{text:'真实原句😀'}}));
  const report={videos:expected.assets.map(a=>({asset_id:a.id,text_analysis:'只解释来源资料，不虚构对白',frames:[{id:`f-${a.id}`,observation:'画面中所见内容与具体静帧绑定'}],limitations:'只看静态采样，音频与连续视频没有观察'})),comparison:'比较表达结构，不把无语音资料当作已识别文案。',idea:{title:'独立选题',viewpoint:'新观点',body:'独立创作内容与来源观察分开，未核验的事实不复用。'},citations:[{asset_id:'a',document_id:'doc-a',quote:'原句😀'}],continuous_video_observed:false,audio_observed:false};
  assert.equal(auditP2(report,calls,expected).actualImages,2);
  for(const mutate of [
    s=>s.calls.find(c=>c.tool==='get_keyframes').result.content=[],
    s=>s.calls.find(c=>c.tool==='get_keyframes').result.structuredContent.frames.items[0].pts++,
    s=>s.calls.find(c=>c.tool==='get_clip').result.structuredContent.clips.items=[],
    s=>s.calls.find(c=>c.tool==='read_transcript').result.structuredContent.reference.document_version=2,
    s=>s.report.citations[0].asset_id='b',
    s=>s.report.videos[0].frames[0].id='unseen',
    s=>s.report.audio_observed=true,
  ]) {const state=structuredClone({report,calls});mutate(state);assert.throws(()=>auditP2(state.report,state.calls,expected));}
});
