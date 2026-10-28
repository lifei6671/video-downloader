import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { Button } from '@/components/ui/button';
import { AlertDialog, AlertDialogContent, AlertDialogHeader, AlertDialogTitle, AlertDialogDescription, AlertDialogFooter, AlertDialogAction, AlertDialogCancel } from '@/components/ui/alert-dialog';

export interface LibraryPendingEditor { dirty: boolean; busy: boolean; canSave: boolean; save: () => Promise<boolean>; discard: () => void }
export function useLibraryEditorNavigation() {
  const editor = useRef<LibraryPendingEditor | null>(null);
  const focus = useRef<HTMLElement | null>(null);
  const [action, setAction] = useState<(() => void) | null>(null);
  const [busy, setBusy] = useState(false);
  const [saving, setSaving] = useState(false);
  const report = useCallback((next: LibraryPendingEditor | null) => { editor.current = next; setBusy(next?.busy ?? false); }, []);
  const request = useCallback((next: () => void) => {
    if (!editor.current?.dirty && !editor.current?.busy) { next(); return; }
    focus.current = document.activeElement as HTMLElement;
    setAction(() => next);
  }, []);
  useEffect(() => {
    if (action && !busy && !saving && !editor.current?.dirty) { setAction(null); action(); }
  }, [action, busy, saving]);
  const dialog = <AlertDialog open={!!action} onOpenChange={open => { if (!open && !busy && !saving) setAction(null); }}>
    <AlertDialogContent onCloseAutoFocus={event => { event.preventDefault(); if (focus.current?.isConnected) focus.current.focus({ preventScroll: true }); }}>
      <AlertDialogHeader><AlertDialogTitle>有未保存的修改</AlertDialogTitle><AlertDialogDescription>当前编辑或冲突时保留的内容尚未保存。</AlertDialogDescription></AlertDialogHeader>
      <AlertDialogFooter>
        <AlertDialogCancel disabled={busy || saving}>继续编辑</AlertDialogCancel>
        <Button type="button" variant="outline" disabled={busy || saving} onClick={event => { event.preventDefault(); const next = action; editor.current?.discard(); setAction(null); next?.(); }}>放弃修改</Button>
        <AlertDialogAction disabled={busy || saving || !editor.current?.canSave} onClick={event => {
          event.preventDefault(); setSaving(true);
          const next = action;
          void editor.current?.save().then(saved => { setAction(null); if (saved) next?.(); }).finally(() => setSaving(false));
        }}>{saving ? '保存中…' : '保存后离开'}</AlertDialogAction>
      </AlertDialogFooter>
    </AlertDialogContent>
  </AlertDialog>;
  return { report, request, dialog };
}
export type LibraryEditorNavigation = ReturnType<typeof useLibraryEditorNavigation>;

/** The detail page can hold both a writing draft and a clip draft. */
export function useLibraryDetailNavigation(navigation: LibraryEditorNavigation) {
  const editors = useRef<[LibraryPendingEditor | null, LibraryPendingEditor | null]>([null, null]);
  const report = navigation.report;
  const update = useCallback((index: 0 | 1, next: LibraryPendingEditor | null) => {
    editors.current[index] = next;
    const values = editors.current.filter((value): value is LibraryPendingEditor => !!value);
    report(values.length ? {
      dirty: values.some(value => value.dirty), busy: values.some(value => value.busy),
      canSave: values.filter(value => value.dirty).every(value => value.canSave),
      save: async () => { for (const value of editors.current) if (value?.dirty && !await value.save()) return false; return true; },
      discard: () => { for (const value of editors.current) value?.discard(); },
    } : null);
  }, [report]);
  const writingReport = useCallback((value: LibraryPendingEditor | null) => update(0, value), [update]);
  const clipsReport = useCallback((value: LibraryPendingEditor | null) => update(1, value), [update]);
  return useMemo(() => ({ writing: { ...navigation, report: writingReport }, clips: { ...navigation, report: clipsReport } }), [navigation, writingReport, clipsReport]);
}
