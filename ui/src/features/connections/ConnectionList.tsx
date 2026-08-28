import { ChevronDown, ChevronRight, Pencil, Power, Trash2 } from 'lucide-react';
import { useEffect, useRef, useState, useSyncExternalStore } from 'react';
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from '../../components/ui/context-menu';
import { Tooltip, TooltipContent, TooltipTrigger } from '../../components/ui/tooltip';
import { getCurrentSessionId, setConnectionDialogOpen, setEditingConnection, subscribe } from '../../lib/app-state';
import { getUngroupedConnections, groupConnections, matchesConnection } from '../../lib/connection-groups';
import sessionManager from '../../lib/session-manager';
import { cn } from '../../lib/utils';
import type { ConnectionManager } from './useConnections';
type Connection = { id: string; name?: string; host?: string; username?: string; group?: string; [key: string]: unknown };

const subscribeCurrentSessionId = (callback: () => void) => subscribe('currentSessionId', callback);

// 拖拽落点用数据描述，由 React 渲染出反馈；不要在运行时拼 Tailwind 类名再 classList.add，
// Tailwind 是编译期扫描源码生成 CSS 的，拼出来的类往往没有对应样式。
type DropTarget =
  | { kind: 'item'; group: string; itemId: string; edge: 'top' | 'bottom'; beforeId: string | null }
  | { kind: 'group'; group: string }
  | { kind: 'ungrouped' };

// 指针追踪高频且不影响渲染，留在 ref 里。
type PointerState = { connectionId: string; pointerId: number; startX: number; startY: number; active: boolean };

type ConnectionListProps = { manager: ConnectionManager; query: string; collapsed?: boolean };

function ConnectionItem({ connection, manager, collapsed = false, dragging = false, dropEdge = null }: {
  connection: Connection;
  manager: ConnectionManager;
  collapsed?: boolean;
  dragging?: boolean;
  dropEdge?: 'top' | 'bottom' | null;
}) {
  const session = sessionManager.getSessionByConnectionId(connection.id);
  const connected = Boolean(session);
  const active = connected && session!.sessionId === getCurrentSessionId();
  const edit = async () => { const all = await window.api.config.getConnections(); const selected = all.find(item => item.id === connection.id); if (selected) { setEditingConnection(selected); setConnectionDialogOpen(true); } };
  const itemDiv = (
        <div
          className={cn(
            'group relative flex min-h-10 cursor-pointer select-none items-center rounded-md py-1.5 text-sm transition-colors',
            collapsed ? 'justify-center px-0' : 'gap-2 px-2',
            active ? 'bg-accent text-accent-foreground' : 'hover:bg-muted',
            connected || 'text-muted-foreground',
            dragging && 'opacity-40',
            // 插入线：看得出会落在这一项的上边还是下边
            dropEdge === 'top' && 'before:absolute before:inset-x-1 before:-top-0.5 before:h-0.5 before:rounded-full before:bg-primary',
            dropEdge === 'bottom' && 'after:absolute after:inset-x-1 after:-bottom-0.5 after:h-0.5 after:rounded-full after:bg-primary',
          )}
          data-connection-item
          data-id={connection.id}
          data-active={active ? 'true' : 'false'}
          data-connected={connected ? 'true' : 'false'}
          onDoubleClick={() => void manager.connectToSaved(connection.id)}
        >
          <span className={`shrink-0 rounded-full ${collapsed ? 'h-2.5 w-2.5 ring-2 ring-border' : 'h-2 w-2'} ${connected ? 'bg-success' : 'bg-muted-foreground/40'}`} data-connection-status />
          {!collapsed && <span className="min-w-0 flex-1 truncate" title={connection.name}>{connection.name}</span>}
          <div className={`shrink-0 items-center gap-0.5 opacity-0 transition-opacity group-hover:opacity-100 group-focus-within:opacity-100 ${collapsed ? 'hidden' : 'flex'}`} data-connection-actions>
            {connected && <button type="button" className="rounded p-1 hover:bg-background" title="断开连接" data-disconnect-connection onClick={event => { event.stopPropagation(); void manager.disconnect(session!.sessionId); }}><Power className="h-3.5 w-3.5" /></button>}
            <button type="button" className="rounded p-1 hover:bg-background disabled:cursor-not-allowed disabled:opacity-40" title={connected ? '断开后才能编辑' : '编辑连接'} disabled={connected} data-edit-connection onClick={event => { event.stopPropagation(); edit(); }}><Pencil className="h-3.5 w-3.5" /></button>
            <button type="button" className="rounded p-1 text-destructive hover:bg-background disabled:cursor-not-allowed disabled:opacity-40" title={connected ? '断开后才能删除' : '删除连接'} disabled={connected} data-delete-connection onClick={event => { event.stopPropagation(); void manager.deleteConnection(connection.id); }}><Trash2 className="h-3.5 w-3.5" /></button>
          </div>
        </div>
  );

  // 两个 Trigger 都用 asChild 链式包住同一个 div，props 才会逐层合并到真实 DOM 上。
  // 若把 <ContextMenu> 整体塞进 TooltipTrigger asChild，hover 事件会落在 Radix Root
  // 组件上而不是 DOM，提示永远不触发。
  const trigger = collapsed
    ? (
      <Tooltip>
        <ContextMenuTrigger asChild>
          <TooltipTrigger asChild>{itemDiv}</TooltipTrigger>
        </ContextMenuTrigger>
        <TooltipContent side="right" sideOffset={8}>{connection.name}</TooltipContent>
      </Tooltip>
    )
    : <ContextMenuTrigger asChild>{itemDiv}</ContextMenuTrigger>;

  return (
    <ContextMenu>
      {trigger}
      <ContextMenuContent>
        <ContextMenuItem onSelect={() => void manager.connectToSaved(connection.id)}>连接</ContextMenuItem>
        <ContextMenuItem disabled={connected} onSelect={edit}>编辑</ContextMenuItem>
        {connected && <ContextMenuItem onSelect={() => void manager.disconnect(session!.sessionId)}>断开连接</ContextMenuItem>}
        <ContextMenuSeparator />
        <ContextMenuItem disabled={connected} className="text-destructive" onSelect={() => void manager.deleteConnection(connection.id)}>删除</ContextMenuItem>
      </ContextMenuContent>
    </ContextMenu>
  );
}

export function ConnectionList({ manager, query, collapsed = false }: ConnectionListProps) {
  useSyncExternalStore(subscribeCurrentSessionId, getCurrentSessionId, () => null);
  const listRef = useRef<HTMLDivElement>(null);
  const pointerRef = useRef<PointerState | null>(null);
  const dropRef = useRef<DropTarget | null>(null);
  const [draggingId, setDraggingId] = useState<string | null>(null);
  const [drop, setDrop] = useState<DropTarget | null>(null);

  const applyDrop = (next: DropTarget | null) => {
    dropRef.current = next;
    setDrop(next);
  };

  // 命中判定顺序：具体项 → 分组头 → 分组区域内的空隙 → 列表内的空白（= 移出分组）。
  // 指针离开列表时返回 null，松手即取消，避免误把连接甩到别处。
  const computeDrop = (clientX: number, clientY: number, draggedId: string): DropTarget | null => {
    const list = listRef.current;
    if (!list) return null;
    const element = document.elementFromPoint(clientX, clientY) as HTMLElement | null;
    if (!element || !list.contains(element)) return null;

    const item = element.closest<HTMLElement>('[data-connection-item]');
    if (item?.dataset.id && item.dataset.id !== draggedId) {
      const group = item.closest<HTMLElement>('[data-connection-group]')?.dataset.group || '';
      const rect = item.getBoundingClientRect();
      const before = clientY < rect.top + rect.height / 2;
      const next = item.nextElementSibling as HTMLElement | null;
      return {
        kind: 'item',
        group,
        itemId: item.dataset.id,
        edge: before ? 'top' : 'bottom',
        beforeId: before ? item.dataset.id : next?.dataset.id ?? null,
      };
    }

    const header = element.closest<HTMLElement>('[data-connection-group-header]');
    if (header) return { kind: 'group', group: header.dataset.group || '' };

    const section = element.closest<HTMLElement>('[data-connection-group]');
    if (section) return { kind: 'group', group: section.dataset.group || '' };

    return { kind: 'ungrouped' };
  };

  const startDrag = (event: React.PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0 || (event.target as HTMLElement).closest('button')) return;
    const item = (event.target as HTMLElement).closest<HTMLElement>('[data-connection-item]');
    if (!item?.dataset.id) return;
    pointerRef.current = { connectionId: item.dataset.id, pointerId: event.pointerId, startX: event.clientX, startY: event.clientY, active: false };
  };

  const moveDrag = (event: React.PointerEvent<HTMLDivElement>) => {
    const pointer = pointerRef.current;
    if (!pointer || pointer.pointerId !== event.pointerId) return;
    // 6px 阈值：低于它算点击，不能一碰就进入拖拽
    const distance = Math.hypot(event.clientX - pointer.startX, event.clientY - pointer.startY);
    if (!pointer.active && distance < 6) return;
    if (!pointer.active) {
      pointer.active = true;
      setDraggingId(pointer.connectionId);
      listRef.current?.setPointerCapture?.(event.pointerId);
    }
    event.preventDefault();
    applyDrop(computeDrop(event.clientX, event.clientY, pointer.connectionId));
  };

  // 收尾挂在 window 上：指针可能在列表外松开，否则拖拽状态会卡住。
  const finishDrag = (event: PointerEvent) => {
    const pointer = pointerRef.current;
    if (!pointer || pointer.pointerId !== event.pointerId) return;
    const target = dropRef.current;
    pointerRef.current = null;
    setDraggingId(null);
    applyDrop(null);
    if (!pointer.active || event.type === 'pointercancel' || !target) return;
    const group = target.kind === 'ungrouped' ? '' : target.group;
    const beforeId = target.kind === 'item' ? target.beforeId : null;
    manager.move(pointer.connectionId, group, beforeId);
  };

  useEffect(() => {
    window.addEventListener('pointerup', finishDrag, true);
    window.addEventListener('pointercancel', finishDrag, true);
    return () => {
      window.removeEventListener('pointerup', finishDrag, true);
      window.removeEventListener('pointercancel', finishDrag, true);
    };
  });

  const visible = manager.connections.filter(connection => matchesConnection(connection, query));
  if (!visible.length) return <div className="p-4 text-center text-xs text-muted-foreground">{manager.connections.length ? '没有匹配的连接' : '没有保存的连接'}</div>;
  const groups = groupConnections(visible, manager.groupOrder);
  const dragging = draggingId !== null;
  const ungrouped = getUngroupedConnections(visible);
  const renderItems = (items: Connection[]) => items.map(connection => (
    <ConnectionItem
      key={connection.id}
      connection={connection}
      manager={manager}
      collapsed={collapsed}
      dragging={draggingId === connection.id}
      dropEdge={drop?.kind === 'item' && drop.itemId === connection.id ? drop.edge : null}
    />
  ));
  return (
    <div ref={listRef} className={`grid select-none gap-1 overflow-y-auto ${collapsed ? 'px-1 py-2' : 'p-2'}`} data-connection-list onPointerDown={startDrag} onPointerMove={moveDrag}>
      <div
        className={cn(
          'grid gap-1 rounded-md transition-colors',
          // 未分组为空时高度为 0，不给它一个最小高度就永远拖不进去
          dragging && 'min-h-10 border border-dashed border-border',
          drop?.kind === 'ungrouped' && 'border-primary bg-primary/10',
        )}
        data-connection-default-items
      >
        {renderItems(ungrouped)}
        {dragging && !ungrouped.length && (
          <div className="px-2 py-2 text-center text-[11px] text-muted-foreground">拖到此处移出分组</div>
        )}
      </div>
      {groups.map(group => {
        const groupCollapsed = manager.isGroupCollapsed(group.name);
        const dropHere = drop?.kind === 'group' && drop.group === group.name;
        return <section key={group.name} data-connection-group data-group={group.name} className={cn('grid gap-1 rounded-md transition-colors', dropHere && 'bg-primary/10 ring-1 ring-primary')}>
          <button type="button" className={cn('flex cursor-pointer select-none items-center rounded py-1 text-left text-xs font-medium text-muted-foreground hover:bg-muted', collapsed ? 'justify-center px-0' : 'gap-1 px-2', dropHere && 'text-primary')} data-connection-group-header data-group={group.name} aria-expanded={!groupCollapsed} title={collapsed ? group.name : undefined} onClick={() => manager.toggleGroup(group.name)}>
            {collapsed
              ? <span className="text-[10px] tabular-nums">{group.connections.length}</span>
              : <>{groupCollapsed ? <ChevronRight className="h-3.5 w-3.5" /> : <ChevronDown className="h-3.5 w-3.5" />}<span className="min-w-0 flex-1 truncate">{group.name}</span><span>{group.connections.length}</span></>}
          </button>
          {!groupCollapsed && <div className={`grid gap-1 ${collapsed ? '' : 'pl-2'}`}>{renderItems(group.connections)}</div>}
        </section>;
      })}
    </div>
  );
}
