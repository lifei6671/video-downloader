// WB-312: actual SDK image blocks, bounded saved media and optional templates.
import assert from 'node:assert/strict';
import { writeFile } from 'node:fs/promises';
import { join, resolve } from 'node:path';
import { spawnSync } from 'node:child_process';

export async function verifyMedia(client, output) {
  const results = [];
  async function call(name, args) {
    const result = await client.callTool({ name, arguments: args });
    assert.notEqual(result.isError, true);
    assert.deepEqual(JSON.parse(result.content[0].text), result.structuredContent);
    assert(!JSON.stringify(result).includes('PRIVATE_'));
    results.push({ name, args, result });
    return result;
  }
  // This first flow deliberately never calls prompts/* or resources/*.
  const clips = (await call('get_clip', { asset_id: 'asset-a' })).structuredContent.clips;
  assert.equal(clips.total, 1);
  const clip = clips.items[0];
  assert.equal(clip.document_id, 'doc-a1');
  assert.equal(clip.document_version, 1);
  assert.equal(clip.start_ms, 100); assert.equal(clip.end_ms, 900);
  await call('read_transcript', { asset_id: 'asset-a', document_id: clip.document_id, version: clip.document_version });
  const first = await call('get_keyframes', { asset_id: 'asset-a', limit: 1 });
  assert.equal(first.structuredContent.frames.items[0].pts, 500);
  assert.equal(first.structuredContent.frames.items[0].time_base_den, 1000);
  const next = await call('get_keyframes', { asset_id: 'asset-a', limit: 1, cursor: first.structuredContent.frames.next_cursor });
  assert.equal(next.structuredContent.frames.next_cursor, null);
  const imageFiles = [];
  for (const [index, result] of [first, next].entries()) {
    const images = result.content.filter(c => c.type === 'image');
    assert.equal(images.length, 1); assert.equal(images[0].mimeType, 'image/png');
    assert.equal(result.structuredContent.frames.items[0].image_status, 'included');
    assert.equal(result.structuredContent.frames.items[0].image_is_preview, false);
    assert.equal(result.structuredContent.frames.items[0].image_width, 16);
    assert.equal(result.structuredContent.frames.items[0].image_height, 16);
    assert(!('image_base64' in result.structuredContent.frames.items[0]));
    const path = join(output, `sdk-image-${index}.png`);
    await writeFile(path, Buffer.from(images[0].data, 'base64')); imageFiles.push(path);
  }
  const decoder = spawnSync(resolve('.venv/bin/python'), ['-c',
    'import json,sys; from PIL import Image; print(json.dumps([{"size":Image.open(p).size,"colors":Image.open(p).convert("RGB").getcolors()} for p in sys.argv[1:]]))', ...imageFiles], { encoding: 'utf8' });
  assert.equal(decoder.status, 0, decoder.stderr);
  const decoded = JSON.parse(decoder.stdout);
  assert.deepEqual(decoded, [{ size: [16,16], colors: [[256,[255,0,0]]] }, { size: [16,16], colors: [[256,[0,255,0]]] }]);
  const both = await call('get_keyframes', { asset_id: 'asset-a' });
  assert.equal(both.content.filter(c => c.type === 'image').length, 2);
  const metadata = await call('get_keyframes', { asset_id: 'asset-a', include_images: false });
  assert.equal(metadata.content.length, 1);
  const range = await call('get_keyframes', { asset_id: 'asset-a', start_ms: 500, end_ms: 750, clip_id: clip.id });
  assert.equal(range.structuredContent.frames.total, 1);
  for (const media_type of ['video', 'audio']) {
    await assert.rejects(client.callTool({ name: 'get_keyframes', arguments: { asset_id: 'asset-a', media_type } }), e => e.code === -32602 && e.message.includes('MEDIA_UNSUPPORTED'));
  }
  await assert.rejects(client.callTool({ name: 'get_clip', arguments: { asset_id: 'asset-b', clip_id: clip.id } }), e => e.message.includes('CLIP_NOT_FOUND'));
  assert.equal((await client.callTool({ name: 'get_keyframes', arguments: { asset_id: 'asset-a', path: '/private' } })).isError, true);
  const prompts = await client.listPrompts();
  assert.deepEqual(prompts.prompts.map(p => p.name), ['analyze_copywriting', 'compare_videos', 'observe_production']);
  for (const p of prompts.prompts) {
    const result = await client.getPrompt({ name: p.name, arguments: { asset_ids: 'asset-a,asset-b' } });
    assert(result.messages[0].content.text.includes('未观察'));
    assert(result.messages[0].content.text.includes('不覆盖人工资料'));
  }
  await assert.rejects(client.getPrompt({ name: 'unknown' }), e => e.message.includes('PROMPT_NOT_FOUND'));
  await assert.rejects(client.getPrompt({ name: 'observe_production', arguments: { path: '/private' } }), e => e.message.includes('INVALID_PROMPT_ARGUMENTS'));
  await assert.rejects(client.listPrompts({ cursor: 'unsupported' }), e => e.message.includes('INVALID_PROMPT_CURSOR'));
  await writeFile(join(output, 'media-results.json'), JSON.stringify({ results, prompts, decoded }, null, 2));
  return { status: 'PASSED', toolOnlyFlow: true, decodedImages: 2, prompts: 3, video: 'MEDIA_UNSUPPORTED', audio: 'MEDIA_UNSUPPORTED' };
}

const object = properties => ({ type: 'object', additionalProperties: false, required: Object.keys(properties), properties });
export const mediaModelSchema = object({
  frames: { type: 'array', minItems: 2, maxItems: 2, items: object({
    id: { type: 'string' }, color: { type: 'string', enum: ['red', 'green', 'unknown'] },
    pts: { type: 'integer' }, time_base_num: { type: 'integer' }, time_base_den: { type: 'integer' },
    sample_start_ms: { type: 'integer' }, sample_end_ms: { type: ['integer','null'] }, batch_id: { type: 'string' },
  }) },
  video_observed: { type: 'boolean' }, audio_observed: { type: 'boolean' },
  limitations: { type: 'string' },
});
export const mediaModelPrompt = `这是 WB-312 合成图片验收，只允许 yingqu 的只读 MCP 工具，不运行命令、不读文件、不调用其他服务、代理或技能，不写回。
目标资产逻辑 ID 是 asset-a。先用 get_clip 读取已有片段及固定文案引用，再用 get_keyframes 读取该资产所有已保存 PNG（include_images=true）。必须依据实际收到的图片内容判断每帧主色，不根据 ID、顺序或文字猜色。工具若没有提供可见图像，请标为 unknown 并说明。
返回 JSON：逐帧记录真实 ID、主色 red/green/unknown、原始 pts/time_base_num/time_base_den、sample_start_ms/sample_end_ms 和 batch_id。说明是否实际消费了连续视频或音频，未提供就为 false；limitations 用中文说明静帧、采样和时间范围的限制，不推断运动、转场、声音或效果。`;
export function auditMediaModel(report, calls) {
  const frames = report.frames.toSorted((a,b) => a.id.localeCompare(b.id));
  assert.deepEqual(frames.map(f => [f.id,f.color,f.pts,f.time_base_num,f.time_base_den,f.sample_start_ms,f.sample_end_ms,f.batch_id]), [
    ['media-frame-0','red',500,1,1000,100,900,'media-batch-0'],
    ['media-frame-1','green',750,1,1000,750,null,'media-batch-1'],
  ]);
  assert.equal(report.video_observed,false); assert.equal(report.audio_observed,false);
  assert(report.limitations.length > 10);
  for (const tool of ['get_clip','get_keyframes']) assert(calls.some(c => c.tool === tool && c.status === 'completed' && !c.error));
  assert(calls.some(c => c.tool === 'get_keyframes' && c.result?.content?.filter(i => i.type === 'image').length === 2), 'Model must actually receive two image blocks');
  return { status: 'PASSED', imagesActuallyDelivered: 2, colorsCorrect: true, samplingMetadataExact: true, video: 'UNSUPPORTED', audio: 'UNSUPPORTED' };
}
