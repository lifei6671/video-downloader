// UI-only persistence semantics. Domain tests exercise SQLite, graph validation and recovery.
export function installAnalysisFixture({ libraryApi, notes, clone, saved, fail, calls, nextTimestamp, waitForSave }) {
  let sequence = 1;
  let restoreFrames;
  const labels = { opening: '开头', viewpoint: '观点', evidence: '论据', turn: '转折', ending: '结尾', takeaways: '可借鉴点' };
  const build = (asset, input, old) => {
    if (input.kind === 'copywriting') return clone(input);
    return { kind: 'technique', observations: input.observations.map(row => {
      if (row.evidence === 'keyframe' && (row.dimension === 'sound' || !row.keyframe_id)) throw 'ANALYSIS_EVIDENCE_INVALID';
      const clip = window.libraryDetailTest.clips().find(c => c.id === row.clip_id && c.asset_id === asset);
      const previous = old?.content.observations.find(r => r.clip_id === row.clip_id && r.keyframe_id === row.keyframe_id);
      if (!clip) { if (!previous) throw 'CLIP_NOT_FOUND'; return { ...clone(previous), ...clone(row) }; }
      const frame = window.keyframesTest.snapshot().find(f => f.id === row.keyframe_id && f.asset_id === asset);
      if (row.keyframe_id && !frame && !previous) throw 'KEYFRAME_NOT_FOUND';
      return { ...clone(row), file_id: clip.file_id, clip_title: clip.title, start_ms: clip.start_ms, end_ms: clip.end_ms, frame_actual_ms: frame?.actual_ms ?? previous?.frame_actual_ms ?? null };
    }) };
  };
  const body = content => content.kind === 'copywriting' ? Object.entries(labels).map(([k, label]) => `## ${label}\n${content[k] || '未记录'}`).join('\n\n') : content.observations.map(row => `${row.dimension} ${row.start_ms}–${row.end_ms}\n${row.text}`).join('\n\n');
  libraryApi.createAnalysis = async (assetId, input) => {
    calls.push({ method: 'createAnalysis', assetId, input: clone(input) }); fail('createAnalysis'); await waitForSave();
    if (input.origin === 'ai' && !input.model_name?.trim()) throw 'ANALYSIS_MODEL_REQUIRED';
    const content = build(assetId, input.content);
    const id = 'analysis-note-' + sequence++;
    notes.set(id, { id, asset_id: assetId, kind: content.kind + '_analysis', body: body(content), origin: input.origin, model_name: input.model_name, citations: clone(input.citations), analysis: { content, human_revised: false }, revision: 1, created_at: nextTimestamp(), updated_at: nextTimestamp() });
    saved('notes', [assetId]); return id;
  };
  libraryApi.editAnalysis = async (assetId, id, content, expectedRevision) => {
    calls.push({ method: 'editAnalysis', assetId, id, content: clone(content), expectedRevision }); fail('editAnalysis'); await waitForSave();
    const note = notes.get(id); if (note?.asset_id !== assetId) throw 'NOTE_NOT_FOUND'; if (note.revision !== expectedRevision) throw 'NOTE_CONFLICT';
    const next = build(assetId, content, note.analysis);
    note.body = body(next); note.analysis = { content: next, human_revised: note.origin === 'ai' || note.analysis.human_revised }; note.revision++; saved('notes', [assetId]);
  };
  window.analysisTest = {
    failResources: value => { if (value) { restoreFrames ??= libraryApi.keyframes; libraryApi.keyframes = async () => { throw 'DATABASE_ERROR'; }; } else if (restoreFrames) { libraryApi.keyframes = restoreFrames; restoreFrames = undefined; } },
    notes: asset => clone([...notes.values()].filter(n => n.asset_id === asset && n.analysis)),
    dropClip: (assetId, id) => libraryApi.deleteClip(assetId, id),
    remoteEdit: id => { const note = notes.get(id); if (note.analysis.content.kind === 'copywriting') note.analysis.content.opening = '另一个窗口的拆解'; else note.analysis.content.observations[0].text = '另一个窗口的观察'; note.body = body(note.analysis.content); note.revision++; saved('notes', [note.asset_id]); },
  };
}
