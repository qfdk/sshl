import { ChevronDown, ChevronRight, Pencil, Power, Trash2 } from 'lucide-react';
import { useEffect, useRef, useSyncExternalStore } from 'react';
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
import type { ConnectionManager } from './useConnections';
type Connection = { id: string; name?: string; host?: string; username?: string; group?: string; [key: string]: unknown };

const subscribeCurrentSessionId = (callback: () => void) => subscribe('currentSessionId', callback);

type DragState = {
  connectionId: string;
  pointerId: number;
  startX: number;
  startY: number;
  active: boolean;
  targetGroup: string | null;
  beforeId: string | null;
  hoverTarget: HTMLElement | null;
  item: HTMLElement;
};

type ConnectionListProps = { manager: ConnectionManager; query: string; collapsed?: boolean };

function ConnectionItem({ connection, manager, collapsed = false }: { connection: Connection; manager: ConnectionManager; collapsed?: boolean }) {
  const session = sessionManager.getSessionByConnectionId(connection.id);
  const connected = Boolean(session);
  const active = connected && session!.sessionId === getCurrentSessionId();
  const edit = async () => { const all = await window.api.config.getConnections(); const selected = all.find(item => item.id === connection.id); if (selected) { setEditingConnection(selected); setConnectionDialogOpen(true); } };
  const itemDiv = (
        <div
          className={`group flex min-h-10 cursor-pointer select-none items-center rounded-md py-1.5 text-sm transition-colors ${collapsed ? 'justify-center px-0' : 'gap-2 px-2'} ${active ? 'bg-accent text-accent-foreground' : 'hover:bg-muted'} ${connected ? '' : 'text-muted-foreground'}`}
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
  const dragRef = useRef<DragState | null>(null);

  const clearHover = () => {
    const drag = dragRef.current;
    drag?.hoverTarget?.classList.remove('bg-primary/15', 'ring-1', 'ring-primary');
    if (drag) drag.hoverTarget = null;
  };

  const finishDrag = (event: PointerEvent) => {
    const drag = dragRef.current;
    if (!drag || drag.pointerId !== event.pointerId) return;
    dragRef.current = null;
    drag.item.classList.remove('opacity-50');
    drag.item.style.pointerEvents = '';
    listRef.current?.classList.remove('select-none');
    clearHover();
    if (!drag.active || event.type === 'pointercancel' || drag.targetGroup === null) return;
    manager.move(drag.connectionId, drag.targetGroup, drag.beforeId);
  };

  useEffect(() => {
    window.addEventListener('pointerup', finishDrag, true);
    window.addEventListener('pointercancel', finishDrag, true);
    return () => {
      window.removeEventListener('pointerup', finishDrag, true);
      window.removeEventListener('pointercancel', finishDrag, true);
    };
  });

  const updateDropTarget = (event: React.PointerEvent<HTMLDivElement>) => {
    const drag = dragRef.current;
    const list = listRef.current;
    if (!drag || !list) return;
    const element = document.elementFromPoint(event.clientX, event.clientY) as HTMLElement | null;
    const targetItem = element?.closest<HTMLElement>('[data-connection-item]');
    const targetHeader = element?.closest<HTMLElement>('[data-connection-group-header]');
    const targetDefault = element?.closest<HTMLElement>('[data-connection-default-items]');
    clearHover();
    drag.targetGroup = null;
    drag.beforeId = null;
    if (targetItem && targetItem.dataset.id !== drag.connectionId) {
      const targetGroup = targetItem.closest<HTMLElement>('[data-connection-group]')?.dataset.group || '';
      const rect = targetItem.getBoundingClientRect();
      const placeBefore = event.clientY < rect.top + rect.height / 2;
      const nextItem = targetItem.nextElementSibling?.closest<HTMLElement>('[data-connection-item]');
      drag.targetGroup = targetGroup;
      drag.beforeId = placeBefore ? targetItem.dataset.id || null : nextItem?.dataset.id || null;
      targetItem.classList.add('bg-primary/15', 'ring-1', 'ring-primary');
      drag.hoverTarget = targetItem;
    } else if (targetHeader) {
      drag.targetGroup = targetHeader.dataset.group || '';
      if (drag.targetGroup && manager.isGroupCollapsed(drag.targetGroup)) manager.toggleGroup(drag.targetGroup);
      targetHeader.classList.add('bg-primary/15', 'ring-1', 'ring-primary');
      drag.hoverTarget = targetHeader;
    } else if (targetDefault) {
      drag.targetGroup = '';
      targetDefault.classList.add('bg-primary/15', 'ring-1', 'ring-primary');
      drag.hoverTarget = targetDefault;
    }
  };

  const startDrag = (event: React.PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0 || (event.target as HTMLElement).closest('button')) return;
    const item = (event.target as HTMLElement).closest<HTMLElement>('[data-connection-item]');
    if (!item?.dataset.id) return;
    dragRef.current = { connectionId: item.dataset.id, pointerId: event.pointerId, startX: event.clientX, startY: event.clientY, active: false, targetGroup: null, beforeId: null, hoverTarget: null, item };
  };

  const moveDrag = (event: React.PointerEvent<HTMLDivElement>) => {
    const drag = dragRef.current;
    if (!drag || drag.pointerId !== event.pointerId) return;
    const distance = Math.hypot(event.clientX - drag.startX, event.clientY - drag.startY);
    if (!drag.active && distance < 6) return;
    if (!drag.active) {
      drag.active = true;
      drag.item.classList.add('opacity-50');
      drag.item.style.pointerEvents = 'none';
      listRef.current?.classList.add('select-none');
      listRef.current?.setPointerCapture?.(event.pointerId);
    }
    event.preventDefault();
    updateDropTarget(event);
  };

  const visible = manager.connections.filter(connection => matchesConnection(connection, query));
  if (!visible.length) return <div className="p-4 text-center text-xs text-muted-foreground">{manager.connections.length ? '没有匹配的连接' : '没有保存的连接'}</div>;
  const groups = groupConnections(visible, manager.groupOrder);
  const renderItems = (items: Connection[]) => items.map(connection => <ConnectionItem key={connection.id} connection={connection} manager={manager} collapsed={collapsed} />);
  return (
    <div ref={listRef} className={`grid select-none gap-1 overflow-y-auto ${collapsed ? 'px-1 py-2' : 'p-2'}`} data-connection-list onPointerDown={startDrag} onPointerMove={moveDrag}>
      <div className="grid gap-1" data-connection-default-items>{renderItems(getUngroupedConnections(visible))}</div>
      {groups.map(group => {
        const groupCollapsed = manager.isGroupCollapsed(group.name);
        return <section key={group.name} data-connection-group data-group={group.name} className="grid gap-1">
          <button type="button" className={`flex cursor-pointer select-none items-center rounded py-1 text-left text-xs font-medium text-muted-foreground hover:bg-muted ${collapsed ? 'justify-center px-0' : 'gap-1 px-2'}`} data-connection-group-header data-group={group.name} aria-expanded={!groupCollapsed} title={collapsed ? group.name : undefined} onClick={() => manager.toggleGroup(group.name)}>
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
