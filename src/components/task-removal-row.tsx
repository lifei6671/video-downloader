import { useLayoutEffect, useRef, type ReactNode } from 'react';

// Keep a successfully removed task visible only while its list space collapses.
export function TaskRemovalRow({ id, removing, onExited, children }: {
  id: string;
  removing: boolean;
  onExited: (id: string) => void;
  children: ReactNode;
}) {
  const row = useRef<HTMLDivElement>(null);
  const focused = useRef(false);
  useLayoutEffect(() => {
    if (!row.current) return;
    const node = row.current;
    if (!removing) { node.inert = false; return; }
    const active = document.activeElement;
    let restoreFocus: (() => void) | undefined;
    if (node.contains(active) || active === document.body && focused.current) {
      const neighbor = [node.nextElementSibling, node.previousElementSibling].find(element => element && !element.hasAttribute('inert'))
        ?? Array.from(node.parentElement?.children ?? []).find(element => element !== node && !element.hasAttribute('inert'));
      const panel = node.closest<HTMLElement>('.task-list-scroll');
      const target = neighbor?.querySelector<HTMLElement>('button:not(:disabled):not([aria-disabled="true"]), [tabindex="0"]') ?? panel;
      restoreFocus = () => {
        if (document.activeElement === document.body || node.contains(document.activeElement)) {
          (target?.isConnected ? target : panel?.isConnected ? panel : null)?.focus({preventScroll:true});
        }
      };
      restoreFocus();
    }
    node.inert = true;
    let done = false;
    const finish = () => { if (!done) { done = true; onExited(id); if (restoreFocus) requestAnimationFrame(restoreFocus); } };
    if (matchMedia('(prefers-reduced-motion: reduce)').matches) {
      finish();
      return;
    }
    const animation = node.animate([
      { height: `${node.getBoundingClientRect().height}px`, paddingBottom: getComputedStyle(node).paddingBottom, opacity: 1, transform: 'translateY(0)' },
      { height: '0px', paddingBottom: '0px', opacity: 0, transform: 'translateY(-12px)' },
    ], { duration: 220, easing: 'ease-out', fill: 'forwards' });
    node.style.overflow = 'hidden';
    animation.finished.then(finish, () => {});
    return () => { animation.cancel(); finish(); };
  }, [id, removing, onExited]);
  return <div ref={row} role="listitem" data-task-row={id} data-removing={removing || undefined} onFocusCapture={() => { focused.current = true; }} onBlurCapture={event => { if (event.relatedTarget) focused.current = event.relatedTarget instanceof Node && event.currentTarget.contains(event.relatedTarget); }} className="task-list-item shrink-0">{children}</div>;
}
