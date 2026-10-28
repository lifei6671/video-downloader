// Synthetic UI state. The Rust suite exercises actual transactions, migrations and packages.
export function installIdeasFixture({ libraryApi, state, saved, clone }) {
  const ideas = new Map(), removedTopics = new Set(); let sequence = 1, failure, blockReads = false, holdId, release;
  const fail = method => { if (failure === method) { failure = undefined; throw 'DATABASE_ERROR'; } };
  const topic = libraryApi.topic, listTopics = libraryApi.topics;
  libraryApi.topic = async id => { if (removedTopics.has(id)) throw 'TOPIC_NOT_FOUND'; return topic(id); };
  libraryApi.topics = async () => (await listTopics()).filter(t => !removedTopics.has(t.id));
  const key = r => JSON.stringify([r.kind, r.asset_id, r.resource_id]);
  async function source(input, previous) {
    fail('prepareIdeaReference');
    if (input.kind === 'topic') {
      try { const value = await libraryApi.topic(input.resource_id); return { reference: clone(input), title: value.topic.title, asset_title: null, missing: false, archived: false, file_state: null, topic_revision: value.topic.revision }; }
      catch (e) { if (!previous || e !== 'TOPIC_NOT_FOUND') throw e; return { ...clone(previous), missing: true, topic_revision: null }; }
    }
    const asset = state.assets.find(a => a.id === input.asset_id);
    let clip;
    if (asset && input.kind === 'clip') clip = (await libraryApi.clips(asset.id)).find(c => c.id === input.resource_id);
    if (!asset || input.kind === 'clip' && !clip) { if (!previous) throw 'IDEA_REFERENCE_INVALID'; return { ...clone(previous), missing: true, archived: !!asset?.archived_at, file_state: null }; }
    if (input.kind === 'video' && asset.id !== input.resource_id) throw 'IDEA_REFERENCE_INVALID';
    return { reference: clone(input), title: clip?.title ?? asset.title, asset_title: asset.title, missing: false, archived: !!asset.archived_at, file_state: clip ? clip.missing ? 'missing' : clip.source_changed ? 'changed' : 'available' : asset.file_state, topic_revision: null };
  }
  const summaries = () => [...ideas.values()].map(v => ({ id: v.idea.id, title: v.idea.title, revision: v.idea.revision, characters: Array.from(v.idea.body).length, reference_count: v.references.length, created_at: v.idea.created_at, updated_at: v.idea.updated_at })).sort((a, b) => b.updated_at - a.updated_at || b.id.localeCompare(a.id));
  libraryApi.ideas = async () => { fail('ideas'); return clone(summaries()); };
  libraryApi.idea = async id => {
    fail('idea'); if (blockReads) throw 'DATABASE_ERROR'; const value = ideas.get(id); if (!value) throw 'IDEA_NOT_FOUND';
    const result = { idea: clone(value.idea), references: await Promise.all(value.references.map(r => source(r.reference, r))) };
    if (holdId === id) { holdId = undefined; await new Promise(resolve => { release = resolve; }); } return result;
  };
  libraryApi.prepareIdeaReference = input => source(input);
  async function build(input, previous) {
    const keys = input.references.map(key); if (new Set(keys).size !== keys.length) throw 'IDEA_REFERENCE_EXISTS';
    return Promise.all(input.references.map(r => source(r, previous?.references.find(old => key(old.reference) === key(r)))));
  }
  libraryApi.createIdea = async input => { fail('createIdea'); const references = await build(input); const id = 'idea-' + sequence++, time = 500 + sequence; ideas.set(id, { idea: { id, title: input.title, viewpoint: input.viewpoint, body: input.body, revision: 1, created_at: time, updated_at: time }, references }); saved('ideas'); return id; };
  libraryApi.editIdea = async (id, input, revision) => { fail('editIdea'); const value = ideas.get(id); if (value?.idea.revision !== revision) throw 'IDEA_CONFLICT'; const references = await build(input, value); Object.assign(value.idea, { title: input.title, viewpoint: input.viewpoint, body: input.body, revision: revision + 1, updated_at: 500 + sequence++ }); value.references = references; saved('ideas'); };
  const workbench = libraryApi.workbench;
  libraryApi.workbench = async () => ({ ...await workbench(), ideas: { items: clone(summaries().slice(0, 6)), total: ideas.size } });
  window.ideasTest = {
    seedAi: (long = false) => { const id = 'ai-idea'; ideas.set(id, { idea: { id, title: 'AI 选题草稿', viewpoint: 'AI 观点', body: 'AI 草稿正文', revision: 1, created_at: 500, updated_at: 500, ai_source: { model_name: long ? 'm'.repeat(128) : 'synthetic-model', citations: [{ asset_id: 'a0', document_id: 'doc-source', quote: long ? 'long-source-token'.repeat(900) : '精确来源原句' }] } }, references: [] }); saved('ideas'); return id; },
    snapshot: () => clone([...ideas.values()]), failNext: method => { failure = method; }, blockReads: value => { blockReads = value; },
    holdNextRead: id => { holdId = id; }, releaseRead: () => release?.(),
    remoteEdit: (id, emit = true) => { const value = ideas.get(id); value.idea.body = '另一个窗口的正文😀'; value.idea.revision++; if (emit) saved('ideas'); },
    archive: id => libraryApi.archive(id, true), deleteClip: (assetId, id) => libraryApi.deleteClip(assetId, id),
    deleteTopic: id => { removedTopics.add(id); saved('topics'); },
    seedSources: async () => {
      const clipId = window.libraryDetailTest.seedClip('a0', 'f-a0', 500, 1250);
      const topicId = await libraryApi.createTopic({ title: '开头研究来源😀', description: '参考专题正文，不能混入原创草稿。' });
      await libraryApi.addTopicMember(topicId, { kind: 'video', asset_id: 'a0', resource_id: 'a0' }, 1);
      await libraryApi.addTopicMember(topicId, { kind: 'clip', asset_id: 'a0', resource_id: clipId }, 2);
      return { clipId, topicId };
    },
  };
}
