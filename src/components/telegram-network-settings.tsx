import { useEffect, useState, type ReactNode } from 'react';
import { api, errorText, type Profile, type Settings } from '@/api';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Card, CardContent, CardFooter, CardHeader, CardTitle } from '@/components/ui/card';
import { Field, FieldContent, FieldDescription, FieldGroup, FieldLabel, FieldSet } from '@/components/ui/field';
import { Checkbox } from '@/components/ui/checkbox';
import { Spinner } from '@/components/ui/spinner';
import { Separator } from '@/components/ui/separator';
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group';
import { toast } from 'sonner';

export function TelegramNetworkSettings({ profile, saved, disabled, children }: { profile: Profile; saved: (value: Settings) => void; disabled: boolean; children: ReactNode }) {
  const [proxy, setProxy] = useState(''), [direct, setDirect] = useState(!profile.proxy);
  const [savedProxy, setSavedProxy] = useState('');
  const [bypassProxy, setBypassProxy] = useState(!!profile.bypass_proxy);
  const [loaded, setLoaded] = useState(false), [busy, setBusy] = useState(false);
  const [error, setError] = useState(''), [retry, setRetry] = useState(0);
  const [proxyError, setProxyError] = useState('');
  const value = direct ? '' : proxy.trim();
  const proxyChanged = !bypassProxy && loaded && value !== savedProxy;
  const changed = proxyChanged || bypassProxy !== !!profile.bypass_proxy;
  const invalidProxy = !bypassProxy && loaded && !direct && !value;
  useEffect(() => {
    let live = true;
    setLoaded(false); setError(''); setProxyError('');
    void api.proxy('telegram').then(value => {
      if (live) { setProxy(value ?? ''); setDirect(!value); setSavedProxy(value?.trim() ?? ''); setLoaded(true); }
    }).catch(error => { if (live) setProxyError(errorText(error)); });
    return () => { live = false; };
  }, [retry]);
  async function save() {
    if (disabled || busy || !changed || invalidProxy) return;
    setBusy(true); setError('');
    try {
      saved(await api.save({ platform: 'telegram', user_agent: profile.user_agent, cookie: null, proxy: proxyChanged ? value : null, bypass_proxy: bypassProxy }));
      if (proxyChanged) setSavedProxy(value);
      toast.success('代理配置已保存');
    } catch (error) { setError(errorText(error)); }
    finally { setBusy(false); }
  }
  return <Card size="sm" className="flex min-w-0 flex-col [--card-spacing:12px]">
    <CardHeader><CardTitle className="font-semibold">Telegram</CardTitle></CardHeader>
    <CardContent className="space-y-4"><form id="profile-telegram" aria-label="Telegram 网络代理" aria-busy={busy} onSubmit={event => { event.preventDefault(); void save(); }}>
      <FieldSet disabled={disabled || busy} className="min-w-0"><FieldGroup className="gap-3">
    <Field orientation="horizontal"><Checkbox id="telegram-bypass-proxy" checked={bypassProxy} onCheckedChange={value => setBypassProxy(value === true)} /><FieldContent><FieldLabel htmlFor="telegram-bypass-proxy">解析与下载不走代理</FieldLabel><FieldDescription>开启 TUN／透明代理时启用，保留已保存的代理地址。</FieldDescription></FieldContent></Field>
    <Field><FieldLabel id="telegram-proxy-mode">网络代理</FieldLabel>
      <ToggleGroup size="sm" type="single" variant="outline" aria-labelledby="telegram-proxy-mode" value={direct ? 'direct' : 'proxy'} disabled={disabled || busy || !loaded || bypassProxy} onValueChange={value => { if (value) setDirect(value === 'direct'); }}>
        <ToggleGroupItem value="direct">直连</ToggleGroupItem><ToggleGroupItem value="proxy">自定义代理</ToggleGroupItem>
      </ToggleGroup>
    </Field>
    {!direct && <Field><FieldLabel htmlFor="telegram-proxy">SOCKS5 地址</FieldLabel><Input id="telegram-proxy" className="min-h-8" type="text" value={proxy} autoComplete="off" spellCheck={false} disabled={disabled || busy || !loaded || bypassProxy} placeholder="socks5://127.0.0.1:7890" onChange={event => setProxy(event.target.value)} /></Field>}
      </FieldGroup></FieldSet>
    {(error || proxyError) && <p role="alert" className="mt-3 text-xs text-destructive">{error || proxyError}</p>}
    </form>
      <Separator />
      <div className="space-y-4">{children}</div>
    </CardContent>
    <CardFooter className="justify-end py-2">
      {!loaded && proxyError && <Button type="button" variant="outline" onClick={() => setRetry(value => value + 1)}>重新读取代理</Button>}
      <Button type="submit" form="profile-telegram" disabled={disabled || busy || !changed || invalidProxy}>{busy && <Spinner aria-hidden="true" data-icon="inline-start" />}{busy ? '保存中…' : '保存代理'}</Button>
    </CardFooter>
  </Card>;
}
