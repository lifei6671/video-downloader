import { useLayoutEffect, useRef, type ReactNode } from 'react';

// Keep a successfully removed task visible only while its list space collapses.
export function TaskRemovalRow({ id, removing, onExited, children }: {
  id: string;
  removing: boolean;
  onExited: (id: string) => void;
  children: ReactNode;
}) {
  const row = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    if (!removing || !row.current) return;
    const node = row.current;
    if (matchMedia('(prefers-reduced-motion: reduce)').matches) {
      onExited(id);
      return;
    }
    let done = false;
    const finish = () => { if (!done) { done = true; onExited(id); } };
    const animation = node.animate([
      { height: `${node.getBoundingClientRect().height}px`, paddingBottom: getComputedStyle(node).paddingBottom, opacity: 1, transform: 'translateY(0)' },
      { height: '0px', paddingBottom: '0px', opacity: 0, transform: 'translateY(-12px)' },
    ], { duration: 220, easing: 'ease-out', fill: 'forwards' });
    node.style.overflow = 'hidden';
    animation.finished.then(finish, () => {});
    return () => { animation.cancel(); finish(); };
  }, [id, removing, onExited]);
  return <div ref={row} role="listitem" data-task-row={id} data-removing={removing || undefined} inert={removing} className="task-list-item shrink-0">{children}</div>;
}
