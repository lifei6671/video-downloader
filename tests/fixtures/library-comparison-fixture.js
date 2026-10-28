// Synthetic UI persistence only. SQLite and backup integrity are tested in Rust.
export function installComparisonFixture({ libraryApi, notes, documents, clone, saved, fail, calls, asset }) {
  const labels = ['opening', 'viewpoint', 'evidence', 'turn', 'ending', 'takeaways'];
  for (const id of ['a0', 'a1']) {
    documents.get('d-' + id).text = `${id} 独有原文😀\n后续内容`;
    documents.get('old-' + id).text = `${id} 旧版原文😀`;
  }
  const content = { kind: 'copywriting', opening: 'a0 独有开头😀', viewpoint: '', evidence: '', turn: '', ending: '', takeaways: '' };
  notes.set('copy-a0', { id: 'copy-a0', asset_id: 'a0', kind: 'copywriting_analysis', body: 'a0 独有开头😀', analysis: { content, human_revised: false }, origin: 'ai', model_name: '合成模型', citations: [], revision: 1, created_at: 300, updated_at: 300 });
  let hold = false, release, seq = 0, failSavedRead = false;
  const readNote = libraryApi.note;
  libraryApi.note = async (assetId, id) => { if (failSavedRead && id.startsWith('compare-')) { failSavedRead = false; throw 'DATABASE_ERROR'; } return readNote(assetId, id); };
  const resolve = async pin => {
    const document = pin.document_id ? await libraryApi.document(pin.asset_id, pin.document_id) : null;
    const analysis = async (ref, kind) => {
      if (!ref) return null;
      const n = await libraryApi.note(pin.asset_id, ref.id);
      if (n.kind !== kind) throw 'COMPARISON_INVALID';
      if (n.revision !== ref.revision) throw 'COMPARISON_SOURCE_CHANGED';
      return { id: n.id, revision: n.revision, origin: n.origin, model_name: n.model_name, human_revised: n.analysis.human_revised, excerpt: n.body, structure: kind === 'copywriting_analysis' ? labels.map(k => n.analysis.content[k]) : null };
    };
    const copywriting = await analysis(pin.copywriting, 'copywriting_analysis'), technique = await analysis(pin.technique, 'technique_analysis');
    const clips = (await libraryApi.clips(pin.asset_id)).filter(c => pin.clip_ids.includes(c.id));
    if (clips.length !== pin.clip_ids.length) throw 'CLIP_NOT_FOUND';
    return clone({ pin, title: asset(pin.asset_id).title, document: document ? { id: document.summary.id, version: document.summary.version, opening: document.text.split('\n')[0] } : null, copywriting, technique, clips });
  };
  libraryApi.comparisonSource = async pin => {
    calls.push({ method: 'comparisonSource', pin: clone(pin) }); fail('comparisonSource'); const result = await resolve(pin);
    if (hold) { hold = false; await new Promise(r => { release = r; }); } return result;
  };
  libraryApi.createComparison = async (assetId, input) => {
    calls.push({ method: 'createComparison', assetId, input: clone(input) }); fail('createComparison');
    const points = await Promise.all(input.points.map(async p => ({ text: p.text, sources: await Promise.all(p.sources.map(resolve)) })));
    const id = 'compare-' + ++seq, comparison = { title: input.title, asset_ids: input.asset_ids, points };
    const citations = points.flatMap(p => p.sources.flatMap(s => s.document ? [{ asset_id: s.pin.asset_id, document_id: s.document.id, quote: s.document.opening }] : []));
    notes.set(id, { id, asset_id: assetId, kind: 'comparison', body: input.title + '\n' + points.map(p => p.text).join('\n'), comparison, citations, origin: 'human', model_name: null, revision: 1, created_at: 500, updated_at: 500 });
    saved('notes', [assetId]); return id;
  };
  libraryApi.editComparison = async (assetId, id, edit, revision) => {
    fail('editComparison'); const n = notes.get(id); if (n.asset_id !== assetId) throw 'NOTE_NOT_FOUND'; if (n.revision !== revision) throw 'NOTE_CONFLICT';
    n.comparison.title = edit.title; n.comparison.points.forEach((p, i) => { p.text = edit.point_texts[i]; }); n.body = edit.title + '\n' + edit.point_texts.join('\n'); n.revision++; saved('notes', [assetId]);
  };
  window.comparisonTest = { failSavedRead: () => { failSavedRead = true; }, seedTechnique: async () => {
    const clip = window.libraryDetailTest.seedClip('a0', 'f-a0', 500, 1250);
    await libraryApi.createAnalysis('a0', { content: { kind: 'technique', observations: [{ dimension: 'visual', evidence: 'video', text: 'a0 画面拆解', clip_id: clip, keyframe_id: null }] }, origin: 'human', model_name: null, citations: [] }); return clip;
  }, holdNextSource: () => { hold = true; }, releaseSource: () => release?.(), notes: () => clone([...notes.values()].filter(n => n.comparison)), remoteEdit: () => { const n = notes.get('copy-a0'); n.analysis.content.opening = 'a0 远端新拆解'; n.body = 'a0 远端新拆解'; n.revision++; saved('notes', ['a0']); } };
}
