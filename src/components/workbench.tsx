import { useCallback, useEffect, useState } from 'react';
import { Download, Library, ListVideo, Plus, RotateCcw } from 'lucide-react';
import { api, errorText } from '@/api';
import { libraryApi, type LibraryDestination, type WorkbenchOperation, type WorkbenchSnapshot } from '@/lib/library-api';
import { LibraryError } from '@/components/library-organization';
import { Button } from '@/components/ui/button';
import { Spinner } from '@/components/ui/spinner';

const phaseNames: Record<string, string> = { extracting_frames: '提取关键帧', parsing: '正在解析', queued: '排队中', downloading: '正在下载', merging: '正在合并', canceling: '正在取消', extracting_audio: '提取音频', recognizing: '正在识别', preparing: '准备中', loading_model: '加载模型', transcribing: '正在识别', saving: '正在保存' };
export function Workbench({ taskSignal, openLibrary, openOperation, navigate }: {
  taskSignal: string; openLibrary: (target: Omit<LibraryDestination, 'sequence'>) => void;
  openOperation: (operation: WorkbenchOperation) => void; navigate: (page: string) => void;
}) {
  const [data, setData] = useState<WorkbenchSnapshot>();
  const [error, setError] = useState('');
  const [subscriptionError, setSubscriptionError] = useState('');
  const [connection, setConnection] = useState(0);
  const [loading, setLoading] = useState(true);
  const [revision, setRevision] = useState(0);
  const refresh = useCallback(() => setRevision(value => value + 1), []);
  useEffect(() => {
    let disposed = false; setSubscriptionError('');
    const stops: (() => void)[] = [];
    const signatures = new Map<string, string>();
    const retain = (stop: () => void) => { if (disposed) stop(); else { stops.push(stop); refresh(); } };
    void libraryApi.subscribe(() => { if (!disposed) refresh(); }).then(retain).catch(reason => { if (!disposed) setSubscriptionError(errorText(reason)); });
    void api.subscribeAssetTranscripts(value => {
      if (disposed) return;
      const key = value.file_id ?? value.task_id;
      const signature = JSON.stringify([value.operation_id, value.state, value.phase, value.error_code]);
      if (signatures.get(key) !== signature) { signatures.set(key, signature); refresh(); }
    }).then(retain).catch(reason => { if (!disposed) setSubscriptionError(errorText(reason)); });
    window.addEventListener('focus', refresh);
    return () => { disposed = true; stops.forEach(stop => stop()); window.removeEventListener('focus', refresh); };
  }, [refresh, connection]);
  useEffect(() => {
    let disposed = false; setLoading(true); setError('');
    void libraryApi.workbench().then(next => { if (!disposed) setData(next); })
      .catch(reason => { if (!disposed) setError(errorText(reason)); })
      .finally(() => { if (!disposed) setLoading(false); });
    return () => { disposed = true; };
  }, [revision, taskSignal]);
  return <section className="workbench" aria-label="工作台内容" aria-busy={loading}>
    <nav className="workbench-entry-links" aria-label="工作台入口">
      <Button variant="outline" onClick={() => openLibrary({ view: 'ideas' })}><Plus />选题与草稿</Button>
      <Button variant="outline" onClick={() => openLibrary({})}><Library />浏览知识库</Button>
      <Button variant="outline" onClick={() => navigate('download')}><Download />下载视频</Button>
      <Button variant="outline" onClick={() => navigate('tasks')}><ListVideo />查看下载任务</Button>
      <Button variant="ghost" size="icon-sm" aria-label="刷新工作台" onClick={refresh} disabled={loading}><RotateCcw /></Button>
    </nav>
    <LibraryError message={error || subscriptionError} />
    {subscriptionError && <Button variant="outline" size="sm" onClick={() => { setConnection(value => value + 1); refresh(); }}>重试连接工作台</Button>}
    {error && <Button variant="outline" size="sm" onClick={refresh}>重试加载工作台</Button>}
    {loading && !data && <div role="status"><Spinner />正在加载</div>}
    {data && <div className="workbench-grid">
      {(['recent', 'uncategorized', 'unorganized'] as const).map(key => <section className="workbench-section" key={key} aria-label={{ recent: '最近访问', uncategorized: '待分类', unorganized: '待整理' }[key]}>
        <div className="workbench-heading"><h2>{{ recent: '最近访问', uncategorized: '待分类', unorganized: '待整理' }[key]}<span>{data[key].total}</span></h2>
          {key !== 'recent' && data[key].total > 0 && <Button variant="ghost" size="sm" onClick={() => openLibrary({ scope: key })}>查看全部</Button>}</div>
        {data[key].items.length ? <ul>{data[key].items.map(asset => <li key={asset.id}><button className="workbench-row" onClick={() => openLibrary({ assetId: asset.id })}><span className="truncate" title={asset.title}>{asset.title}</span>{asset.author && <span className="workbench-row-meta truncate">{asset.author}</span>}</button></li>)}</ul> : <p className="workbench-empty">{{ recent: '暂无访问记录', uncategorized: '暂无待分类视频', unorganized: '暂无待整理视频' }[key]}</p>}
      </section>)}
      {data.ideas && <section className="workbench-section" aria-label="最近编辑的选题"><div className="workbench-heading"><h2>最近编辑的选题<span>{data.ideas.total}</span></h2>{data.ideas.total > 0 && <Button size="sm" variant="ghost" onClick={() => openLibrary({ view: 'ideas' })}>查看全部选题</Button>}</div>{data.ideas.items.length ? <ul>{data.ideas.items.map(idea => <li key={idea.id}><button className="workbench-row" aria-label={`编辑选题 ${idea.title}`} onClick={() => openLibrary({ view: 'ideas', ideaId: idea.id })}><span className="truncate" title={idea.title}>{idea.title}</span></button></li>)}</ul> : <p className="workbench-empty">暂无已保存选题</p>}</section>}
      <section className="workbench-section" aria-label="运行中操作">
        <div className="workbench-heading"><h2>运行中操作<span>{data.operations.filter(item => item.state === 'running').length}</span></h2></div>
        {data.operations.some(item => item.state === 'running') ? <ul>{data.operations.filter(item => item.state === 'running').map(item => <li key={`${item.kind}:${item.id}`}><Operation item={item} open={openOperation} /></li>)}</ul> : <p className="workbench-empty">暂无运行中操作</p>}
        {data.operations.some(item => item.state !== 'running') && <><h3 className="workbench-attention">需要处理</h3><ul>{data.operations.filter(item => item.state !== 'running').map(item => <li key={`${item.kind}:${item.id}`}><Operation item={item} open={openOperation} /></li>)}</ul></>}
      </section>
    </div>}
  </section>;
}
function Operation({ item, open }: { item: WorkbenchOperation; open: (operation: WorkbenchOperation) => void }) {
  return <button className="workbench-row" onClick={() => open(item)}>
    <span className="truncate" title={item.title}>{item.title}</span>
    <span className="workbench-row-meta">{{ keyframes: '关键帧', download: '下载', transcription: '提取文案', import: '导入', backup: '备份', restore: '恢复' }[item.kind]}{item.state === 'running' ? `，${phaseNames[item.phase ?? ''] ?? '处理中'}` : item.state === 'interrupted' ? '，已中断' : '，失败'}</span>
    {item.error && <span className="workbench-operation-error">{errorText(item.error)}</span>}
  </button>;
}
