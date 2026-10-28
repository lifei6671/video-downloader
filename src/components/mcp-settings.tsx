import { useCallback, useEffect, useRef, useState } from 'react';
import { Copy, RotateCcw } from 'lucide-react';
import { Switch as SwitchPrimitive } from 'radix-ui';
import { mcpApi, type McpConnection, type McpConfigFormat } from '@/lib/mcp-api';
import { Button } from '@/components/ui/button';
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group';
import { Spinner } from '@/components/ui/spinner';

const messages: Record<string, string> = {
  MCP_ADAPTER_MISSING: '未找到 MCP 适配程序，请确认应用目录中包含 yingqu-mcp。',
  MCP_ADAPTER_PATH_INVALID: 'MCP 适配程序路径无法用于客户端配置。',
  IPC_BIND_FAILED: 'MCP 服务启动失败，请重试。',
  IPC_SERVICE_FAILED: 'MCP 服务已异常停止，请重新开启。',
  MCP_CONFIG_UNAVAILABLE: '连接配置已失效，请检查服务状态后重新复制。',
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
  const [format, setFormat] = useState<McpConfigFormat>('codex');
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
      if (action === 'copy') { await mcpApi.copyConfig(format); setCopied(true); }
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
  const adapterError = info?.adapter_error ? failure(info.adapter_error) : '';
  return <div hidden={!active} className="mcp-settings">
    <section aria-label="MCP 服务" className="mcp-service">
      <div className="mcp-service-row">
        <div><label htmlFor="mcp-enabled" className="text-sm font-medium">启用 MCP</label><p role="status" className="text-xs text-muted-foreground">{status}</p></div>
        <SwitchPrimitive.Root id="mcp-enabled" className="mcp-switch" checked={enabled} disabled={loading || !!pending || !info || (!enabled && !info.adapter_path)} onCheckedChange={value => void perform(value ? 'enable' : 'disable')}><SwitchPrimitive.Thumb className="mcp-switch-thumb" /></SwitchPrimitive.Root>
      </div>
      <p className="mcp-hint">本机 AI 客户端可读取知识库，所读取的内容可能发送至客户端使用的模型服务。</p>
      <div className="mcp-service-row">
        <div><label htmlFor="mcp-write-enabled" className="text-sm font-medium">允许 AI 写入</label><p role="status" className="text-xs text-muted-foreground">{pending === 'write-on' || pending === 'write-off' ? '正在更新权限…' : info?.write_enabled && enabled ? '已允许写入' : '仅允许读取'}</p></div>
        <SwitchPrimitive.Root id="mcp-write-enabled" className="mcp-switch" checked={enabled && !!info?.write_enabled} disabled={!enabled || loading || !!pending || !info} onCheckedChange={value => void perform(value ? 'write-on' : 'write-off')}><SwitchPrimitive.Thumb className="mcp-switch-thumb" /></SwitchPrimitive.Root>
      </div>
      <p className="mcp-hint">可新增笔记、分类、专题和草稿，追加分类标签，以及启动和取消转录。关闭服务后恢复只读。</p>
      <p className="mcp-hint">每次启动默认关闭；关闭服务或退出映取会断开连接。仅支持本机 stdio 客户端。</p>
      {(error || serviceError || adapterError) && <div role="alert" className="mcp-error">{[...new Set([error, serviceError, adapterError].filter(Boolean))].map(message => <p key={message}>{message}</p>)}</div>}
      <Button className="self-start" variant="outline" size="sm" disabled={loading || !!pending} onClick={() => void refresh()}>{loading ? <Spinner /> : <RotateCcw />}刷新状态</Button>
    </section>
    <section aria-label="MCP 客户端配置" className="mcp-client">
      <h3 className="text-sm font-semibold">客户端配置</h3>
      <ToggleGroup type="single" variant="outline" aria-label="配置格式" value={format} disabled={!!pending} onValueChange={value => { if (value === 'codex' || value === 'json') { setFormat(value); setCopied(false); } }}>
        <ToggleGroupItem value="codex">Codex（TOML）</ToggleGroupItem><ToggleGroupItem value="json">通用 JSON</ToggleGroupItem>
      </ToggleGroup>
      {info?.configs ? <pre tabIndex={0} aria-label="客户端配置内容" className="mcp-config">{info.configs[format]}</pre> : <p className="mcp-hint">{enabled ? '连接配置不可用，请检查服务状态。' : '开启服务后显示连接配置。'}</p>}
      <div className="flex items-center gap-3"><Button size="sm" disabled={!info?.configs || loading || !!pending} onClick={() => void perform('copy')}>{pending === 'copy' ? <Spinner /> : <Copy />}复制配置</Button><span role="status" className="text-xs text-muted-foreground">{copied ? '配置已复制' : ''}</span></div>
    </section>
  </div>;
}
