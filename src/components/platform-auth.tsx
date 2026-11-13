import { useEffect, useState } from 'react';
import { api, errorText, type Profile, type AccountAuth } from '@/api';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Field, FieldDescription, FieldError, FieldLabel } from '@/components/ui/field';
import { Spinner } from '@/components/ui/spinner';
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs';
import { Textarea } from '@/components/ui/textarea';

const siteNames: Record<string, string> = { weixin: '腾讯元宝', douyin: '抖音', xiaohongshu: '小红书', tiktok: 'TikTok', youtube: 'YouTube' };

export type AccountAuthActions = {
  pending: AccountAuth | null;
  login: () => void;
  clearLogin: () => void;
  apply: (mode: AccountAuth, cookie?: string) => Promise<boolean>;
};

export function PlatformAuthSettings({ profile, actions }: { profile: Profile; actions: AccountAuthActions }) {
  const name = siteNames[profile.platform];
  const external = profile.platform === 'youtube';
  const loginLabel = profile.platform === 'weixin' ? '元宝登录' : '内置登录';
  const active = profile.account_auth ?? null;
  const [view, setView] = useState<AccountAuth | null>(null);
  const selected = view ?? active ?? (external ? 'manual' : 'login');
  const [draft, setDraft] = useState<string | null>(null);
  const [saved, setSaved] = useState('');
  const [readError, setReadError] = useState('');
  const [loaded, setLoaded] = useState(false);
  const busy = actions.pending !== null;

  useEffect(() => {
    if (selected !== 'manual' || loaded) return;
    let disposed = false;
    setReadError('');
    api.cookie(profile.platform).then(value => {
      if (!disposed) {
        setSaved(value);
        setDraft(null);
        setLoaded(true);
      }
    }).catch(error => { if (!disposed) setReadError(errorText(error)); });
    return () => { disposed = true; };
  }, [loaded, selected, profile.platform]);

  async function saveManual(value: string) {
    if (await actions.apply('manual', value)) {
      if (value === '') {
        setSaved('');
        setDraft(null);
      } else {
        setLoaded(false);
      }
    }
  }

  const cookie = draft ?? saved;
  const manualChanged = cookie.trim() !== saved;
  const manualAction = active === 'manual' ? '保存 Cookie' : manualChanged ? '保存并切换' : '切换到手动 Cookie';
  return <Field className="gap-3">
    <div className="flex flex-wrap items-center justify-between gap-2">
      <FieldLabel id={`${profile.platform}-auth-label`}>认证方式</FieldLabel>
      <div className="flex items-center gap-2">
        <span className="flex size-4 items-center justify-center" role="status">
          {busy && <><Spinner aria-hidden="true" /><span className="sr-only">正在处理认证设置…</span></>}
        </span>
        <Badge variant="secondary" role="status">当前使用：{active === 'login' ? loginLabel : active === 'manual' ? '手动 Cookie' : '匿名访问'}</Badge>
      </div>
    </div>
    <Tabs value={selected} onValueChange={value => setView(value as AccountAuth)} className="gap-3" aria-busy={busy}>
      {!external && <TabsList aria-labelledby={`${profile.platform}-auth-label`} className="grid h-auto w-full grid-cols-2">
        <TabsTrigger value="login" disabled={busy}>{loginLabel}</TabsTrigger>
        <TabsTrigger value="manual" disabled={busy}>手动 Cookie</TabsTrigger>
      </TabsList>}
      {!external && <TabsContent value="login" className="flex min-w-0 flex-wrap items-center justify-between gap-3 data-[state=inactive]:hidden">
        <FieldDescription>{profile.login_configured ? '已保存登录会话' : profile.platform === 'tiktok'
          ? '请用已登录的 TikTok 手机客户端扫码。'
          : profile.platform === 'weixin' ? '使用微信扫码或快捷登录。' : `在官方页面登录${name}。`}</FieldDescription>
        <div className="flex flex-wrap gap-2">
          {profile.login_configured && <Button type="button" size="sm" variant="ghost" disabled={busy} onClick={actions.clearLogin}>清除会话</Button>}
          <Button type="button" size="sm" variant={profile.login_configured ? 'outline' : 'default'} disabled={busy} onClick={actions.login}>
            {profile.login_configured ? '重新登录' : `登录${name === 'TikTok' ? ' TikTok' : name}`}
          </Button>
          {profile.login_configured && active !== 'login' &&
            <Button type="button" size="sm" disabled={busy} onClick={() => void actions.apply('login')}>切换到{loginLabel}</Button>}
        </div>
      </TabsContent>}
      <TabsContent value="manual" className="min-w-0 data-[state=inactive]:hidden">
        <Field className="gap-2">
          {external && <div className="flex flex-wrap items-center justify-between gap-2">
            <FieldDescription>Google 登录需在浏览器完成。登录后可粘贴 YouTube Cookie。</FieldDescription>
            <Button type="button" variant="outline" size="sm" onClick={actions.login}>打开 YouTube 登录</Button>
          </div>}
          <FieldLabel htmlFor={profile.platform + '-cookie'}>{profile.platform === 'weixin' ? '腾讯元宝网页 Cookie' : '网页端 Cookie'}</FieldLabel>
          <Textarea id={profile.platform + '-cookie'} rows={4} spellCheck={false}
            className="field-sizing-fixed h-24 min-h-24 max-h-48 resize-y break-all"
            placeholder={loaded ? '粘贴网页 Cookie' : readError ? 'Cookie 读取失败' : '正在读取 Cookie…'}
            value={cookie} disabled={busy || !loaded} onChange={event => setDraft(event.target.value)} />
          {readError && <FieldError role="alert">{readError}</FieldError>}
          <div className="flex flex-wrap justify-end gap-2">
            {profile.cookie_configured && <Button type="button" size="sm" variant="ghost" disabled={busy || !loaded} onClick={() => void saveManual('')}>清除 Cookie</Button>}
            <Button type="button" size="sm" disabled={busy || !loaded || !cookie.trim() || (active === 'manual' && !manualChanged)}
              onClick={() => void (manualChanged ? saveManual(cookie.trim()) : actions.apply('manual'))}>
              {manualAction}
            </Button>
          </div>
        </Field>
      </TabsContent>
    </Tabs>
  </Field>;
}
