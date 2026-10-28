// UI-only reference graph. Domain tests validate real SQLite transactions and packages.
export function installTopicsFixture({ libraryApi, state, saved, asset, clone }) {
  const topics = new Map(), members = new Map(), missingNotes = new Set(); let sequence = 1, failure, failReads = false;
  const fail = method => { if (failure === method) { failure = undefined; throw 'DATABASE_ERROR'; } };
  const requireTopic = (id, revision) => { const t = topics.get(id); if (!t) throw 'TOPIC_NOT_FOUND'; if (revision != null && t.revision !== revision) throw 'TOPIC_CONFLICT'; return t; };
  const label = n => n.body.split('\n').find(s => s.trim()) ?? '笔记';
  const view = m => {
    const a = state.assets.find(a => a.id === m.asset_id); let value, file_state = null, note_revision = null;
    if (m.kind === 'video') { value = a; file_state = a?.file_state === 'changed' ? 'changed' : a?.file_state === 'missing' ? 'missing' : 'available'; }
    else if (m.kind === 'clip') { value = window.libraryDetailTest.clips().find(c => c.id === m.resource_id && c.asset_id === m.asset_id); const file = window.libraryDetailTest.snapshot(m.asset_id).files.find(f => f.id === value?.file_id); file_state = !file || file.missing ? 'missing' : file.changed ? 'changed' : 'available'; }
    else { value = missingNotes.has(m.resource_id) ? null : window.writingTest.snapshot(m.asset_id).notes.find(n => n.id === m.resource_id); note_revision = value?.revision ?? null; }
    return { member: clone(m), title: value ? m.kind === 'note' ? label(value) : value.title : m.label, asset_title: a?.title ?? m.asset_title, missing: !a || !value, archived: !!a?.archived_at, file_state, note_revision };
  };
  libraryApi.topics = async () => { fail('topics'); return clone([...topics.values()]); };
  libraryApi.topic = async id => { fail('topic'); if (failReads) throw 'DATABASE_ERROR'; return clone({ topic: requireTopic(id), members: [...members.values()].filter(m => m.topic_id === id).sort((a, b) => a.position - b.position).map(view) }); };
  libraryApi.createTopic = async input => { fail('createTopic'); const id = 'topic-' + sequence++; topics.set(id, { id, ...clone(input), revision: 1, created_at: 500, updated_at: 500 }); saved('topics'); return id; };
  libraryApi.editTopic = async (id, input, revision) => { fail('editTopic'); const t = requireTopic(id, revision); Object.assign(t, clone(input)); t.revision++; saved('topics'); };
  libraryApi.addTopicMember = async (id, input, revision) => {
    fail('addTopicMember'); const t = requireTopic(id, revision), a = asset(input.asset_id);
    if ([...members.values()].some(m => m.topic_id === id && m.kind === input.kind && m.resource_id === input.resource_id)) throw 'TOPIC_MEMBER_EXISTS';
    const m = { ...clone(input), id: 'member-' + sequence++, topic_id: id, asset_title: a.title, label: a.title, position: [...members.values()].filter(m => m.topic_id === id).length, created_at: 500 };
    const resolved = view(m); if (resolved.missing) throw 'TOPIC_REFERENCE_INVALID'; m.label = resolved.title; members.set(m.id, m); t.revision++; saved('topics'); return m.id;
  };
  libraryApi.removeTopicMember = async (id, memberId, revision) => {
    fail('removeTopicMember'); const t = requireTopic(id, revision); if (members.get(memberId)?.topic_id !== id) throw 'TOPIC_MEMBER_NOT_FOUND';
    members.delete(memberId); [...members.values()].filter(m => m.topic_id === id).sort((a, b) => a.position - b.position).forEach((m, i) => { m.position = i; }); t.revision++; saved('topics');
  };
  libraryApi.reorderTopicMembers = async (id, ids, revision) => { fail('reorderTopicMembers'); const t = requireTopic(id, revision); if (new Set(ids).size !== ids.length || ids.some(i => members.get(i)?.topic_id !== id)) throw 'TOPIC_INVALID'; ids.forEach((m, i) => { members.get(m).position = i; }); t.revision++; saved('topics'); };
  window.topicsTest = {
    seedAi: () => { const id = 'ai-topic'; topics.set(id, { id, title: 'AI 研究专题', description: 'AI 专题正文', revision: 1, created_at: 500, updated_at: 500, ai_source: { model_name: 'synthetic-model', citations: [{ asset_id: 'a0', document_id: 'doc-source', quote: '精确来源原句' }] } }); saved('topics'); return id; },
    archive: id => libraryApi.archive(id, true), deleteClip: (assetId, id) => libraryApi.deleteClip(assetId, id),
    snapshot: () => clone({ topics: [...topics.values()], members: [...members.values()] }),
    failNext: method => { failure = method; }, failReads: value => { failReads = value; },
    remoteEdit: (id, emit = true) => { const t = topics.get(id); t.title = '另一个窗口的专题'; t.revision++; if (emit) saved('topics'); },
    deleteNote: id => { missingNotes.add(id); saved('notes'); },
    seedClip: () => window.libraryDetailTest.seedClip('a0', 'f-a0', 500, 1250),
  };
}
