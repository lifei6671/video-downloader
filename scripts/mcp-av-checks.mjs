import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { writeFile } from 'node:fs/promises';
import { join, resolve } from 'node:path';

export async function verifyAudiovisual(client, core, profile, output) {
  const python = resolve(process.platform === 'win32' ? '.venv/Scripts/python.exe' : '.venv/bin/python');
  const generated = spawnSync(python, ['-c', `import sys,subprocess;sys.path.insert(0,sys.argv[2]);from local_tools import installed_tool;subprocess.run([installed_tool('ffmpeg'),'-nostdin','-v','error','-f','lavfi','-i','color=c=red:s=160x90:r=10:d=18','-f','lavfi','-i','sine=frequency=440:duration=18','-c:v','libx264','-c:a','aac','-shortest',sys.argv[1]],check=True)`, join(profile, 'av.mp4'), resolve('python-engine')], { encoding: 'utf8', windowsHide: true });
  assert.equal(generated.status, 0, generated.stderr);
  const item = (await core.request('import_av')).result;
  const args = { asset_id: item.asset_id, file_id: item.file_id, start_ms: 0, end_ms: 15000 };
  const audio = await client.callTool({ name: 'get_audio_excerpt', arguments: args });
  assert.notEqual(audio.isError, true);
  const blocks = audio.content.filter(c => c.type === 'audio');
  assert.equal(blocks.length, 1); assert.equal(blocks[0].mimeType, 'audio/wav');
  assert.equal(audio.structuredContent.frames, 240000);
  assert(!('audio_base64' in audio.structuredContent));
  assert(Buffer.byteLength(JSON.stringify(audio)) < 1024 * 1024);
  assert(!JSON.stringify(audio).includes(profile));
  const path = join(output, 'sdk-audio.wav'); await writeFile(path, Buffer.from(blocks[0].data, 'base64'));
  const decoded = spawnSync(python, ['-c', 'import wave,sys,json;f=wave.open(sys.argv[1]);print(json.dumps(dict(frames=f.getnframes(),rate=f.getframerate(),channels=f.getnchannels(),width=f.getsampwidth())))', path], { encoding: 'utf8', windowsHide: true });
  assert.equal(decoded.status, 0, decoded.stderr); assert.deepEqual(JSON.parse(decoded.stdout), { frames: 240000, rate: 16000, channels: 1, width: 2 });
  await core.request('write_permission', { enabled: true });
  const run = (await client.callTool({ name: 'start_frame_sampling', arguments: args })).structuredContent;
  assert(run.operation_id); assert.equal(run.total, 5);
  let terminal;
  const deadline = Date.now() + 20000;
  while (Date.now() < deadline) {
    terminal = (await client.callTool({ name: 'get_frame_sampling_status', arguments: { operation_id: run.operation_id } })).structuredContent;
    if (terminal.state !== 'running') break;
    await new Promise(resolve => setTimeout(resolve, 50));
  }
  assert.equal(terminal.state, 'completed', JSON.stringify(terminal));
  const images = await client.callTool({ name: 'get_keyframes', arguments: { asset_id: item.asset_id, file_id: item.file_id, limit: 5 } });
  assert.equal(images.content.filter(c => c.type === 'image').length, 5);
  for (const [i, frame] of images.content.filter(c => c.type === 'image').entries()) await writeFile(join(output, `sdk-range-${i}.png`), Buffer.from(frame.data, 'base64'));
  assert(images.structuredContent.frames.items.every(f => f.sample_count === 5 && !f.source_changed && f.sample_start_ms === 0 && f.sample_end_ms === 15000));
  await core.request('write_permission', { enabled: false });
  return { status: 'PASSED', officialSdkAudioReceived: true, wavDecoded: true, fiveRealPngsReceived: true, operationId: run.operation_id, actualModelAudioConsumption: 'NOT_RUN', assetId: item.asset_id, fileId: item.file_id };
}
