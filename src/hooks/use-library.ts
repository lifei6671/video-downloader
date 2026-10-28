import { useCallback, useEffect, useRef, useState } from 'react';
import { errorText } from '@/api';
import { libraryApi, type LibraryCategory, type LibrarySearchPage, type LibrarySearchRequest, type LibraryTag } from '@/lib/library-api';

export function useLibrary(request: LibrarySearchRequest, active: boolean) {
  const [page, setPage] = useState<LibrarySearchPage>({ items: [], next_cursor: null });
  const [categories, setCategories] = useState<LibraryCategory[]>([]);
  const [tags, setTags] = useState<LibraryTag[]>([]);
  const [loading, setLoading] = useState(true);
  const [moreLoading, setMoreLoading] = useState(false);
  const [error, setError] = useState('');
  const [subscriptionError, setSubscriptionError] = useState('');
  const [organizationReady, setOrganizationReady] = useState(false);
  const [revision, setRevision] = useState(0);
  const generation = useRef(0);
  const morePending = useRef(false);
  const key = JSON.stringify(request);
  const refresh = useCallback(() => setRevision(value => value + 1), []);

  useEffect(() => {
    if (!active) return;
    let disposed = false;
    let unsubscribe: (() => void) | undefined;
    setSubscriptionError('');
    void libraryApi.subscribe(() => { if (!disposed) refresh(); }).then(stop => {
      if (disposed) stop(); else { unsubscribe = stop; refresh(); }
    }).catch(reason => { if (!disposed) setSubscriptionError(errorText(reason)); });
    window.addEventListener('focus', refresh);
    return () => { disposed = true; unsubscribe?.(); window.removeEventListener('focus', refresh); };
  }, [active, refresh]);

  useEffect(() => {
    const current = ++generation.current;
    if (!active) return;
    setLoading(true); setMoreLoading(false); morePending.current = false; setError('');
    void Promise.allSettled([libraryApi.search(JSON.parse(key)), libraryApi.categories(), libraryApi.tags()]).then(([next, groups, labels]) => {
      if (generation.current !== current) return;
      if (next.status === 'fulfilled') setPage(next.value);
      else { setPage({ items: [], next_cursor: null }); setError(errorText(next.reason)); }
      if (groups.status === 'fulfilled') setCategories(groups.value); else setError(errorText(groups.reason));
      if (labels.status === 'fulfilled') setTags(labels.value); else setError(errorText(labels.reason));
      setOrganizationReady(groups.status === 'fulfilled' && labels.status === 'fulfilled');
      setLoading(false);
    });
    return () => { generation.current++; };
  }, [key, active, revision]);

  async function loadMore() {
    if (loading || morePending.current || !page.next_cursor) return;
    const current = generation.current;
    morePending.current = true; setMoreLoading(true); setError('');
    try {
      const next = await libraryApi.search({ ...request, cursor: page.next_cursor });
      if (current !== generation.current) return;
      setPage(old => {
        const ids = new Set(old.items.map(item => item.asset.id));
        return { items: [...old.items, ...next.items.filter(item => !ids.has(item.asset.id))], next_cursor: next.next_cursor };
      });
    } catch (reason) { if (current === generation.current) setError(errorText(reason)); }
    finally { if (current === generation.current) { morePending.current = false; setMoreLoading(false); } }
  }
  return { ...page, categories, tags, revision, organizationReady, loading, moreLoading, error: error || subscriptionError, refresh, loadMore };
}
