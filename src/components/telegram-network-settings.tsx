import { useEffect, useState } from 'react';
import { api, errorText, type Profile, type Settings } from '@/api';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Field, FieldLabel } from '@/components/ui/field';
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group';

export function TelegramNetworkSettings({ profile, saved, disabled }: { profile: Profile; saved: (value: Settings) => void; disabled: boolean }) {
  const [proxy, setProxy] = useState(''), [direct, setDirect] = useState(!profile.proxy);
  const [loaded, setLoaded] = useState(false), [busy, setBusy] = useState(false);
  const [error, setError] = useState(''), [notice, setNotice] = useState(''), [retry, setRetry] = useState(0);
  useEffect(() => {
    let live = true;
    setLoaded(false); setError('');
    void api.proxy('telegram').then(value => {
      if (live) { setProxy(value ?? ''); setDirect(!value); setLoaded(true); }
    }).catch(error => { if (live) setError(errorText(error)); });
    return () => { live = false; };
  }, [retry]);
  async function save() {
    if (disabled || busy || !loaded) return;
    setBusy(true); setError(''); setNotice('');
    try {
      saved(await api.save({ platform: 'telegram', user_agent: profile.user_agent, cookie: null, proxy: direct ? '' : proxy.trim() }));
      setNotice('代理配置已保存');
    } catch (error) { setError(errorText(error)); }
    finally { setBusy(false); }
  }
  return <form aria-label="Telegram 网络代理" className="space-y-3" onSubmit={event => { event.preventDefault(); void save(); }}>
    <Field><FieldLabel id="telegram-proxy-mode">网络代理</FieldLabel>
      <ToggleGroup size="sm" type="single" variant="outline" aria-labelledby="telegram-proxy-mode" value={direct ? 'direct' : 'proxy'} disabled={disabled || busy || !loaded} onValueChange={value => { if (value) { setDirect(value === 'direct'); setNotice(''); } }}>
        <ToggleGroupItem value="direct">直连</ToggleGroupItem><ToggleGroupItem value="proxy">自定义代理</ToggleGroupItem>
      </ToggleGroup>
    </Field>
    {!direct && <Field><FieldLabel htmlFor="telegram-proxy">SOCKS5 地址</FieldLabel><Input id="telegram-proxy" value={proxy} autoComplete="off" spellCheck={false} disabled={disabled || busy || !loaded} placeholder="socks5://127.0.0.1:7890" onChange={event => { setProxy(event.target.value); setNotice(''); }} /></Field>}
    <div className="flex gap-2"><Button type="submit" variant="outline" disabled={disabled || busy || !loaded || !direct && !proxy.trim()}>{busy ? '正在保存…' : '保存代理'}</Button>
      {!loaded && error && <Button type="button" variant="outline" onClick={() => setRetry(value => value + 1)}>重新读取代理</Button>}
    </div>
    {error && <p role="alert" className="text-xs text-destructive">{error}</p>}
    {notice && <p role="status" className="text-xs text-muted-foreground">{notice}</p>}
  </form>;
}
