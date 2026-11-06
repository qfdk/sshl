import { Plus, X } from 'lucide-react';
import { useEffect, useRef, useState, useSyncExternalStore, type PointerEvent as ReactPointerEvent } from 'react';
import { getCurrentSessionId, setConnectionDialogOpen, setEditingConnection, subscribe } from '../lib/app-state';
import { pointerMoveStep } from '../lib/pointer-release';
import sessionManager from '../lib/session-manager';
import { applyTabOrder, moveTabId, toSessionTabs } from '../lib/session-tabs';
import { shallowEqual } from '../lib/shallow-equal';
import { cn } from '../lib/utils';
import type { ConnectionManager } from '../features/connections/useConnections';

const subscribeCurrentSessionId = (callback: () => void) => subscribe('currentSessionId', callback);

type DropMark = { id: string; side: 'before' | 'after' };
// 指针追踪高频且不影响渲染，留在 ref 里（与侧边栏拖拽同一套做法：指针事件而非 HTML5 拖放）。
type PointerState = { id: string; pointerId: number; startX: number; startY: number; lastX: number; lastY: number; active: boolean };

export function SessionTabs({ manager }: { manager: ConnectionManager }) {
  const currentSessionId = useSyncExternalStore(subscribeCurrentSessionId, getCurrentSessionId, () => null);
  const [order, setOrder] = useState<string[]>([]);
  const [draggingId, setDraggingId] = useState<string | null>(null);
  const [mark, setMark] = useState<DropMark | null>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const pointerRef = useRef<PointerState | null>(null);
  const markRef = useRef<DropMark | null>(null);
  const suppressClickRef = useRef(false);
  // sessionManager 不是响应式的；会话增删都会 bump manager.revision，App 随之重渲染，这里跟着读到最新。
  const tabs = applyTabOrder(toSessionTabs(sessionManager.getAllSessions(), manager.connections, currentSessionId), order);

  // 落点没变（同一个标签的同一侧）不 setState：指针每动一下都重渲染整排标签没有必要
  const applyMark = (next: DropMark | null) => {
    if (shallowEqual(markRef.current, next)) return;
    markRef.current = next;
    setMark(next);
  };

  const startDrag = (event: ReactPointerEvent<HTMLDivElement>, id: string) => {
    if (event.button !== 0 || (event.target as HTMLElement).closest('[data-session-tab-close]')) return;
    pointerRef.current = { id, pointerId: event.pointerId, startX: event.clientX, startY: event.clientY, lastX: event.clientX, lastY: event.clientY, active: false };
  };

  const moveDrag = (event: ReactPointerEvent<HTMLDivElement>) => {
    const pointer = pointerRef.current;
    if (!pointer || pointer.pointerId !== event.pointerId) return;
    // 5px 阈值：低于它算点击（切换会话），不能一碰就进入拖拽
    const step = pointerMoveStep(pointer, event, Math.hypot(event.clientX - pointer.startX, event.clientY - pointer.startY), 5);
    if (step === 'ignore') return;
    if (step !== 'drag') {
      // 左键已松开却没收到 pointerup（WKWebView 常丢）：见 pointerMoveStep
      if (pointer.active) listRef.current?.releasePointerCapture?.(event.pointerId);
      if (step === 'drop') finishDrag(event.nativeEvent);
      else clearDrag();
      return;
    }
    if (!pointer.active) {
      pointer.active = true;
      setDraggingId(pointer.id);
      listRef.current?.setPointerCapture?.(event.pointerId);
    }
    pointer.lastX = event.clientX;
    pointer.lastY = event.clientY;
    const target = (document.elementFromPoint(event.clientX, event.clientY) as HTMLElement | null)?.closest<HTMLElement>('[data-session-tab]');
    const targetId = target?.dataset.sessionTab;
    if (!target || !targetId || targetId === pointer.id) {
      applyMark(null);
      return;
    }
    const rect = target.getBoundingClientRect();
    applyMark({ id: targetId, side: event.clientX < rect.left + rect.width / 2 ? 'before' : 'after' });
  };

  const clearDrag = () => {
    pointerRef.current = null;
    setDraggingId(null);
    applyMark(null);
  };

  const finishDrag = (event: PointerEvent) => {
    const pointer = pointerRef.current;
    if (!pointer || pointer.pointerId !== event.pointerId) return;
    const target = markRef.current;
    clearDrag();
    if (!pointer.active) return;
    // 拖完松手紧接着的 click 不算切换会话
    suppressClickRef.current = true;
    setTimeout(() => { suppressClickRef.current = false; }, 0);
    if (event.type === 'pointercancel' || !target) return;
    setOrder(moveTabId(tabs.map(tab => tab.sessionId), pointer.id, target.id, target.side));
  };
  // 监听只挂一次，经 ref 调到最新一次渲染的 finishDrag（要用当时的 tabs），不必每次渲染都拆了重挂
  const finishDragRef = useRef(finishDrag);
  finishDragRef.current = finishDrag;

  // 收尾挂在 window 上：指针可能在标签栏外松开，否则拖拽状态会卡住。
  useEffect(() => {
    const finish = (event: PointerEvent) => finishDragRef.current(event);
    window.addEventListener('pointerup', finish, true);
    window.addEventListener('pointercancel', finish, true);
    return () => {
      window.removeEventListener('pointerup', finish, true);
      window.removeEventListener('pointercancel', finish, true);
    };
  }, []);

  return (
    <div ref={listRef} data-tauri-drag-region role="tablist" aria-label="已打开的会话" className="flex min-w-0 select-none flex-wrap items-center gap-1.5" onPointerMove={moveDrag}>
      {tabs.map(tab => (
        <div
          key={tab.sessionId}
          data-session-tab={tab.sessionId}
          className={cn(
            'relative flex h-[34px] items-center rounded-[10px] border',
            tab.active ? 'border-border bg-card shadow-sm' : 'border-transparent hover:bg-foreground/[0.04]',
            draggingId === tab.sessionId && 'opacity-50',
            // 落点竖线：看得出会插到这个标签的左边还是右边
            mark?.id === tab.sessionId && mark.side === 'before' && 'before:absolute before:-left-1 before:inset-y-1 before:w-0.5 before:rounded-full before:bg-primary',
            mark?.id === tab.sessionId && mark.side === 'after' && 'after:absolute after:-right-1 after:inset-y-1 after:w-0.5 after:rounded-full after:bg-primary',
          )}
          onPointerDown={event => startDrag(event, tab.sessionId)}
        >
          {/* 走 connectToSaved 而不是 switchToSession：会话已失效时它会接着重连，和侧边栏双击一致 */}
          <button type="button" role="tab" aria-selected={tab.active} className="flex h-full min-w-0 items-center gap-2 pl-3 pr-1.5 text-sm" onClick={() => { if (!tab.active && !suppressClickRef.current) void manager.connectToSaved(tab.connectionId); }}>
            <span className="h-[7px] w-[7px] shrink-0 rounded-full bg-success" />
            <span className={cn('truncate', tab.active ? 'font-semibold' : 'text-foreground/80')}>{tab.name}</span>
            {tab.active && tab.host && <span className="truncate font-mono text-[11px] text-muted-foreground">{tab.host}</span>}
          </button>
          <button type="button" data-session-tab-close className="mr-1 grid h-[26px] w-[26px] shrink-0 place-items-center rounded-[7px] text-muted-foreground hover:bg-foreground/[0.06] hover:text-foreground" title={`断开 ${tab.name}`} aria-label={`断开 ${tab.name}`} onClick={() => void manager.disconnect(tab.sessionId)}>
            <X className="h-3 w-3" />
          </button>
        </div>
      ))}
      {!tabs.length && <span data-tauri-drag-region className="px-1 text-sm text-muted-foreground">未连接</span>}
      {/* 没有会话时侧边栏的「新建连接」就够了，这里只在已有标签时提供 + */}
      {tabs.length > 0 && <button type="button" className="grid h-[34px] w-[34px] place-items-center rounded-[10px] border border-dashed border-foreground/20 text-muted-foreground hover:border-foreground/35 hover:text-foreground" title="新建连接" aria-label="新建连接" onClick={() => { setEditingConnection(null); setConnectionDialogOpen(true); }}>
        <Plus className="h-3.5 w-3.5" />
      </button>}
    </div>
  );
}
