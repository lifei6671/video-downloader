import { useState, type RefObject } from 'react';
import { api } from '@/api';
import { toast } from 'sonner';
import { InputGroupTextarea } from '@/components/ui/input-group';
import { ContextMenu, ContextMenuTrigger, ContextMenuContent, ContextMenuItem, ContextMenuSeparator } from '@/components/ui/context-menu';

export function ShareLinkInput({ value, onChange, inputRef, disabled = false }: {
  value: string;
  onChange: (value: string) => void;
  inputRef: RefObject<HTMLTextAreaElement | null>;
  disabled?: boolean;
}) {
  const [hasSelection, setHasSelection] = useState(false);
  const [clipboardBusy, setClipboardBusy] = useState(false);

  async function edit(action: 'selectAll' | 'cut' | 'copy' | 'paste' | 'clear') {
    const field = inputRef.current;
    if (!field || field.disabled || clipboardBusy) return;
    const { value: original, selectionStart: start, selectionEnd: end } = field;
    if (action === 'selectAll') {
      requestAnimationFrame(() => { if (inputRef.current === field) { field.focus(); field.select(); } });
      return;
    }
    setClipboardBusy(true);
    try {
      let replacement = '';
      if (action === 'copy' || action === 'cut') {
        if (start === end) return;
        await navigator.clipboard.writeText(original.slice(start, end));
        if (action === 'copy') return;
      } else if (action === 'paste') {
        replacement = (await api.readClipboardText()).replace(/\r\n?/g, '\n');
        if (!replacement) { toast.info('剪贴板中没有文本内容'); return; }
      }
      // A pending clipboard operation must not overwrite newer input or another page.
      if (inputRef.current !== field || field.disabled || field.value !== original) return;
      const next = action === 'clear' ? '' : original.slice(0, start) + replacement + original.slice(end);
      const caret = action === 'clear' ? 0 : start + replacement.length;
      onChange(next);
      requestAnimationFrame(() => {
        if (inputRef.current === field) { field.focus(); field.setSelectionRange(caret, caret); }
      });
    } catch {
      toast.error(action === 'paste' ? '无法读取剪贴板，请使用 Ctrl / ⌘ + V 粘贴。' : '无法写入剪贴板，请使用 Ctrl / ⌘ + C 或 X。');
    } finally { setClipboardBusy(false); }
  }

  return <ContextMenu onOpenChange={open => {
    if (open) { const field = inputRef.current; setHasSelection(!!field && field.selectionStart !== field.selectionEnd); }
  }}><ContextMenuTrigger asChild><InputGroupTextarea id="video-input" ref={inputRef} rows={3}
    className="field-sizing-fixed min-h-0 select-text overflow-y-auto" value={value} disabled={disabled} readOnly={clipboardBusy}
    onChange={event => onChange(event.target.value)} onKeyDown={event => {
      if (event.key === 'Enter' && !event.shiftKey && !event.nativeEvent.isComposing) {
        event.preventDefault(); event.currentTarget.form?.requestSubmit();
      }
    }} placeholder="粘贴作品分享链接或分享口令…" /></ContextMenuTrigger>
    <ContextMenuContent aria-label="作品链接编辑" onCloseAutoFocus={event => { event.preventDefault(); inputRef.current?.focus(); }}>
      <ContextMenuItem disabled={disabled || clipboardBusy || !value} onSelect={() => void edit('selectAll')}>全选</ContextMenuItem>
      <ContextMenuSeparator />
      <ContextMenuItem disabled={disabled || clipboardBusy || !hasSelection} onSelect={() => void edit('cut')}>剪切</ContextMenuItem>
      <ContextMenuItem disabled={disabled || clipboardBusy || !hasSelection} onSelect={() => void edit('copy')}>复制</ContextMenuItem>
      <ContextMenuItem disabled={disabled || clipboardBusy} onSelect={() => void edit('paste')}>粘贴</ContextMenuItem>
      <ContextMenuSeparator />
      <ContextMenuItem disabled={disabled || clipboardBusy || !value} onSelect={() => void edit('clear')}>清空</ContextMenuItem>
    </ContextMenuContent>
  </ContextMenu>;
}
