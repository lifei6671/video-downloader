import { useCallback, useEffect, useRef, useState } from 'react';
import { Copy, RotateCcw } from 'lucide-react';
import { Switch as SwitchPrimitive } from 'radix-ui';
import { mcpApi, type McpConnection } from '@/lib/mcp-api';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Spinner } from '@/components/ui/spinner';

const messages: Record<string, string> = {
  MCP_HTTP_BIND_FAILED: 'MCP 服务启动失败，本机端口可能已被占用。',
  MCP_HTTP_SERVICE_FAILED: 'MCP 服务已异常停止，请重新开启。',
  IPC_BIND_FAILED: 'MCP 服务启动失败，请重试。',
  IPC_SERVICE_FAILED: 'MCP 服务已异常停止，请重新开启。',
  MCP_CONNECTION_UNAVAILABLE: '请先开启 MCP 服务后再复制地址。',
  CLIPBOARD_WRITE_FAILED: '复制失败，请重试。',
  MCP_SERVICE_DISABLED: '请先开启 MCP 服务。',
  INVALID_STATE: '应用正在退出，无法开启 MCP 服务。',
};
const failure = (error: unknown) => messages[String(error)] ?? '操作失败，请重试。';

export function McpSettings({ active }: { active: boolean }) {
  const [info, setInfo] = useState<McpConnection | null>(null);
  const [loading, setLoading] = useState(false);
  const [pending, setPending] = useState<'enable' | 'disable' | 'copy' | 'write-on' | 'write-off' | null>(null);
  const [error, setError] = useState('');
  const [copied, setCopied] = useState(false);
  const request = useRef(0);
  const changing = useRef(false);
  const refresh = useCallback(async (silent = false) => {
    if (changing.current) return;
    const id = ++request.current;
    if (!silent) { setLoading(true); setError(''); setCopied(false); }
    try {
      const next = await mcpApi.connection();
      if (id === request.current) setInfo(next);
    } catch {
      if (id === request.current) { setInfo(null); setError('无法读取 MCP 状态，请重试。'); }
    } finally {
      if (id === request.current) setLoading(false);
    }
  }, []);
  useEffect(() => {
    if (!active) return;
    void refresh();
    const timer = window.setInterval(() => void refresh(true), 3000);
    const focus = () => void refresh(true);
    window.addEventListener('focus', focus);
    return () => { ++request.current; window.clearInterval(timer); window.removeEventListener('focus', focus); };
  }, [active, refresh]);
  const perform = async (action: 'enable' | 'disable' | 'copy' | 'write-on' | 'write-off') => {
    if (changing.current) return;
    changing.current = true;
    ++request.current;
    setLoading(false); setPending(action); setError(''); setCopied(false);
    try {
      if (action === 'copy') { await mcpApi.copyUrl(); setCopied(true); }
      else if (action === 'write-on' || action === 'write-off') setInfo(await mcpApi.setWriteEnabled(action === 'write-on'));
      else setInfo(await mcpApi.setEnabled(action === 'enable'));
    } catch (reason) {
      setError(failure(reason));
      try { setInfo(await mcpApi.connection()); } catch { setInfo(null); }
    } finally { changing.current = false; setPending(null); }
  };
  const enabled = info?.status.enabled ?? false;
  const status = pending === 'enable' ? '正在开启…' : pending === 'disable' ? '正在关闭…' : loading ? '正在读取…' : !info ? '状态不可用' : info.status.error ? '服务异常' : enabled ? '运行中' : '已关闭';
  const serviceError = info?.status.error ? failure(info.status.error) : '';
  return <div hidden={!active} className="mcp-settings">
    <section aria-label="MCP 服务" className="mcp-service">
      <div className="mcp-service-row">
        <div><label htmlFor="mcp-enabled" className="text-sm font-medium">启用 MCP</label><p role="status" className="text-xs text-muted-foreground">{status}</p></div>
        <SwitchPrimitive.Root id="mcp-enabled" className="mcp-switch" checked={enabled} disabled={loading || !!pending || !info} onCheckedChange={value => void perform(value ? 'enable' : 'disable')}><SwitchPrimitive.Thumb className="mcp-switch-thumb" /></SwitchPrimitive.Root>
      </div>
      <p className="mcp-hint">本机 AI 客户端可读取素材库，所读取的内容可能发送至客户端使用的模型服务。</p>
      <div className="mcp-service-row">
        <div><label htmlFor="mcp-write-enabled" className="text-sm font-medium">允许 AI 写入</label><p role="status" className="text-xs text-muted-foreground">{pending === 'write-on' || pending === 'write-off' ? '正在更新权限…' : info?.write_enabled && enabled ? '已允许写入' : '仅允许读取'}</p></div>
        <SwitchPrimitive.Root id="mcp-write-enabled" className="mcp-switch" checked={enabled && !!info?.write_enabled} disabled={!enabled || loading || !!pending || !info} onCheckedChange={value => void perform(value ? 'write-on' : 'write-off')}><SwitchPrimitive.Thumb className="mcp-switch-thumb" /></SwitchPrimitive.Root>
      </div>
      <p className="mcp-hint">可新增笔记和分类、添加分类标签，以及提取文案和采样画面。关闭服务后恢复只读。</p>
      <p className="mcp-hint">仅允许本机连接。每次启动默认关闭，关闭服务或退出映取后连接失效。</p>
      {(error || serviceError) && <div role="alert" className="mcp-error">{[...new Set([error, serviceError].filter(Boolean))].map(message => <p key={message}>{message}</p>)}</div>}
      <Button className="self-start" variant="outline" size="sm" disabled={loading || !!pending} onClick={() => void refresh()}>{loading ? <Spinner /> : <RotateCcw />}刷新状态</Button>
    </section>
    <section aria-label="MCP 连接地址" className="mcp-client">
      <h3 className="text-sm font-semibold">连接地址</h3>
      {enabled && info?.status.endpoint ? <Input readOnly aria-label="MCP 连接地址" value={info.status.endpoint} /> : <p className="mcp-hint">开启服务后显示连接地址。</p>}
      <div className="flex items-center gap-3"><Button size="sm" disabled={!enabled || !info?.status.endpoint || loading || !!pending} onClick={() => void perform('copy')}>{pending === 'copy' ? <Spinner /> : <Copy />}复制地址</Button><span role="status" className="text-xs text-muted-foreground">{copied ? '地址已复制' : ''}</span></div>
    </section>
  </div>;
}
