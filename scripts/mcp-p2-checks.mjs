// WB-313: actual saved media, model evidence and additive write-back.
import assert from 'node:assert/strict';
import { content } from './mcp-client-checks.mjs';

const object = properties => ({type:'object',additionalProperties:false,required:Object.keys(properties),properties});
export const p2Schema = object({
  videos:{type:'array',minItems:2,maxItems:2,items:object({
    asset_id:{type:'string'}, text_analysis:{type:'string'},
    frames:{type:'array',minItems:1,items:object({id:{type:'string'},observation:{type:'string'}})},
    limitations:{type:'string'},
  })},
  comparison:{type:'string'},
  idea:object({title:{type:'string'},viewpoint:{type:'string'},body:{type:'string'}}),
  citations:{type:'array',minItems:1,items:object({asset_id:{type:'string'},document_id:{type:'string'},quote:{type:'string'}})},
  continuous_video_observed:{type:'boolean'},audio_observed:{type:'boolean'},
});
export function p2Prompt(expected) {
  return `这是 WB-313 实际视频学习流程验收。只使用 yingqu 的只读 MCP 工具，不运行命令、不读文件、不用网页、其他服务、代理或技能，不直接写入。
本轮资产逻辑 ID：${expected.assets.map(a=>a.id).join(', ')}。只读取这两个资产。每个都调用 get_asset、get_clip、get_keyframes（include_images=true，limit=1，沿 next_cursor 分页直到六张图全部实际收到），并调用 list_notes / read_note 读取已有笔记正文。图像若是 image_is_preview=true，必须明确它是缩小预览，保留其原始 PTS/采样与实际预览尺寸，细字无法辨认则写未知。
对存在文案的资产，先 get_asset 得到固定 document_id / version，再 read_transcript；没有文案或无语音的资产明确未知，不能把关键帧文字编成对白。
比较两种开场的信息组织、文案与静帧呈现，引用真正读到的原句；只依据实际收到的采样画面，不能声称看过连续视频、转场或听过声音，也不确认画面事件真实性或原文统计事实。
提出一个自己的新选题，写清独立观点和短草稿，借鉴表达方式而不复制来源、数字或事件。每条视频返回实际消费帧的ID和具体画面观察，并说明无语音、语音段粗定位、预览分辨率和有限采样等限制。按 schema 输出中文 JSON。无需讨论对话标题或操作权限。`;
}
export function auditP2(report,calls,expected) {
  assert.equal(report.continuous_video_observed,false); assert.equal(report.audio_observed,false);
  assert.equal(new Set(report.videos.map(v=>v.asset_id)).size,2);
  for(const asset of expected.assets) {
    const video=report.videos.find(v=>v.asset_id===asset.id);assert(video);
    assert(video.text_analysis.length>5 && video.limitations.length>10);
    const successful=calls.filter(c=>c.status==='completed'&&!c.error);
    assert(successful.some(c=>c.tool==='get_asset'&&c.arguments.asset_id===asset.id));
    assert(successful.some(c=>c.tool==='get_clip'&&c.arguments.asset_id===asset.id&&content(c.result).clips.items.some(clip=>clip.id===asset.frames[0].clip_id)));
    const consumed=new Map();
    for(const call of successful.filter(c=>c.tool==='get_keyframes'&&c.arguments.asset_id===asset.id)) {
      const result=call.result;const metadata=content(result);const included=metadata.frames.items.filter(f=>f.image_status==='included');
      assert.equal(included.length,result.content.filter(c=>c.type==='image').length);
      for(const frame of included) {
        const saved=asset.frames.find(f=>f.id===frame.id);assert(saved);
        for(const key of ['pts','time_base_num','time_base_den','sample_start_ms','sample_end_ms','batch_id','file_id','clip_id']) assert.deepEqual(frame[key],saved[key]);
        consumed.set(frame.id,frame);
      }
    }
    assert.equal(consumed.size,asset.frames.length,'Every saved sample must actually arrive as an image');
    assert.equal(new Set(video.frames.map(f=>f.id)).size,asset.frames.length);
    for(const observation of video.frames) {assert(consumed.has(observation.id));assert(observation.observation.length>8);}
  }
  for(const citation of report.citations) {
    const asset=expected.assets.find(a=>a.id===citation.asset_id);assert(asset?.document);
    assert.equal(citation.document_id,asset.document.id);assert(citation.quote.length>0&&asset.document.text.includes(citation.quote));
    assert(calls.some(c=>c.tool==='read_transcript'&&c.status==='completed'&&!c.error&&content(c.result).reference.asset_id===asset.id&&content(c.result).reference.document_id===citation.document_id&&content(c.result).reference.document_version===asset.document.version&&content(c.result).content.text.includes(citation.quote)));
  }
  assert(report.comparison.length>20&&report.idea.title.length>0&&report.idea.body.length>20);
  return {status:'PASSED',assets:2,actualImages:expected.assets.reduce((n,a)=>n+a.frames.length,0),citations:report.citations.length,semanticQuality:'REQUIRES_REVIEW'};
}
export async function writeP2(call,report,expected,modelName) {
  const source={model_name:modelName,citations:report.citations};const notes=[];
  for(const video of report.videos) {
    const asset=expected.assets.find(a=>a.id===video.asset_id);
    const sampling=asset.frames.map(f=>({id:f.id,pts:f.pts,time_base_num:f.time_base_num,time_base_den:f.time_base_den,sample_start_ms:f.sample_start_ms,sample_end_ms:f.sample_end_ms,batch_id:f.batch_id}));
    const body=`WB313 实际采样学习\n\n${video.text_analysis}\n\n${video.frames.map(f=>`${f.id}: ${f.observation}`).join('\n')}\n\n比较：${report.comparison}\n\n限制：${video.limitations}\n未观察连续视频或音频。\n采样依据：${JSON.stringify(sampling)}`;
    const created=content(await call('create_note',{asset_id:asset.id,body,source}));assert.equal(created.origin,'ai');
    const stored=content(await call('read_note',{note_id:created.id,length:16000}));assert.equal(stored.content.text,body);
    notes.push({asset_id:asset.id,id:created.id,body});
  }
  const references=expected.assets.map(a=>({kind:'clip',asset_id:a.id,resource_id:a.frames[0].clip_id}));
  const input={...report.idea,references};const idea=content(await call('create_idea',{input,source}));assert.equal(idea.origin,'ai');
  return {status:'PASSED',notes,idea:{id:idea.id,input},source};
}
