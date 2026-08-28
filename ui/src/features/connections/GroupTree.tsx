import { ChevronDown, ChevronRight, GripVertical, Trash2 } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { Input } from '../../components/ui/input';
import { Button } from '../../components/ui/button';
import { UNGROUPED_LABEL, normalizeGroupName } from '../../lib/connection-groups';
import { cn } from '../../lib/utils';

export type TreeConnection = { id: string; name?: string; host?: string; username?: string; group?: string };

type Props = {
  /** 顺序即显示顺序；归属读 connection.group，空串为未分组 */
  connections: TreeConnection[];
  groups: string[];
  onMoveConnection: (connectionId: string, targetGroup: string, beforeId: string | null) => void;
  onReorderGroups: (next: string[]) => void;
  onRenameGroup: (oldName: string, newName: string) => void;
  onRemoveGroup: (name: string) => void;
};

// 落点用数据描述、由 React 渲染反馈。不要在运行时拼 Tailwind 类再 classList.add：
// Tailwind 编译期扫描源码，运行时才拼出的类名没有对应 CSS。
type DropTarget =
  | { kind: 'intoGroup'; group: string }
  | { kind: 'beside'; group: string; itemId: string; edge: 'top' | 'bottom'; beforeId: string | null }
  | { kind: 'groupOrder'; before: string | null; marker: string };

type Drag =
  | { kind: 'connection'; id: string; pointerId: number; startX: number; startY: number; active: boolean }
  | { kind: 'group'; name: string; pointerId: number; startY: number; active: boolean };

const DRAG_THRESHOLD = 6;

export function GroupTree({ connections, groups, onMoveConnection, onReorderGroups, onRenameGroup, onRemoveGroup }: Props) {
  const rootRef = useRef<HTMLDivElement>(null);
  const dragRef = useRef<Drag | null>(null);
  const dropRef = useRef<DropTarget | null>(null);
  const [drag, setDrag] = useState<Drag | null>(null);
  const [drop, setDrop] = useState<DropTarget | null>(null);
  // 默认全部折叠：这样整棵树就是一列分组，拖起来最好排。展开某个分组后才排它的机器。
  const [expanded, setExpanded] = useState<Set<string>>(new Set());

  const setDropTarget = (next: DropTarget | null) => {
    dropRef.current = next;
    setDrop(next);
  };

  const membersOf = (group: string) =>
    connections.filter(connection => normalizeGroupName(connection.group) === group);

  const toggle = (group: string) =>
    setExpanded(previous => {
      const next = new Set(previous);
      if (next.has(group)) next.delete(group);
      else next.add(group);
      return next;
    });

  // 指针离开树时返回 null，松手即取消，不会把东西甩到别处。
  const computeDrop = (clientX: number, clientY: number, current: Drag): DropTarget | null => {
    const root = rootRef.current;
    if (!root) return null;
    const element = document.elementFromPoint(clientX, clientY) as HTMLElement | null;
    if (!element || !root.contains(element)) return null;

    const section = element.closest<HTMLElement>('[data-tree-group]');

    // 拖分组：只认分组区块，落在自己身上不算；未分组固定在顶部，不参与排序。
    if (current.kind === 'group') {
      const name = section?.dataset.treeGroup;
      if (!name || name === current.name) return null;
      const rect = section!.getBoundingClientRect();
      const after = clientY >= rect.top + rect.height / 2;
      const index = groups.indexOf(name);
      return { kind: 'groupOrder', before: after ? groups[index + 1] ?? null : name, marker: name };
    }

    // 拖机器：优先落在具体机器行上（组内/跨组排序），否则并入所在分组末尾。
    const row = element.closest<HTMLElement>('[data-tree-connection]');
    if (row?.dataset.treeConnection && row.dataset.treeConnection !== current.id) {
      const group = row.closest<HTMLElement>('[data-tree-group]')?.dataset.treeGroup ?? '';
      const rect = row.getBoundingClientRect();
      const before = clientY < rect.top + rect.height / 2;
      const next = row.nextElementSibling as HTMLElement | null;
      return {
        kind: 'beside',
        group,
        itemId: row.dataset.treeConnection,
        edge: before ? 'top' : 'bottom',
        beforeId: before ? row.dataset.treeConnection : next?.dataset.treeConnection ?? null,
      };
    }
    return { kind: 'intoGroup', group: section?.dataset.treeGroup ?? '' };
  };

  const beginConnectionDrag = (event: React.PointerEvent, id: string) => {
    if (event.button !== 0) return;
    dragRef.current = { kind: 'connection', id, pointerId: event.pointerId, startX: event.clientX, startY: event.clientY, active: false };
  };

  const beginGroupDrag = (event: React.PointerEvent, name: string) => {
    if (event.button !== 0) return;
    event.preventDefault();
    dragRef.current = { kind: 'group', name, pointerId: event.pointerId, startY: event.clientY, active: false };
  };

  const handleMove = (event: React.PointerEvent) => {
    const current = dragRef.current;
    if (!current || current.pointerId !== event.pointerId) return;
    // 阈值之下算点击，否则轻碰一下就误进入拖拽
    const distance = current.kind === 'connection'
      ? Math.hypot(event.clientX - current.startX, event.clientY - current.startY)
      : Math.abs(event.clientY - current.startY);
    if (!current.active && distance < DRAG_THRESHOLD) return;
    if (!current.active) {
      current.active = true;
      setDrag(current);
      rootRef.current?.setPointerCapture?.(event.pointerId);
    }
    event.preventDefault();
    setDropTarget(computeDrop(event.clientX, event.clientY, current));
  };

  // 收尾挂 window：指针可能在树外松开，容器级监听收不到，拖拽状态会卡住。
  const finish = (event: PointerEvent) => {
    const current = dragRef.current;
    if (!current || current.pointerId !== event.pointerId) return;
    const target = dropRef.current;
    dragRef.current = null;
    setDrag(null);
    setDropTarget(null);
    if (!current.active || event.type === 'pointercancel' || !target) return;

    if (current.kind === 'connection') {
      if (target.kind === 'beside') onMoveConnection(current.id, target.group, target.beforeId);
      else if (target.kind === 'intoGroup') onMoveConnection(current.id, target.group, null);
      return;
    }
    if (target.kind === 'groupOrder') {
      const next = groups.filter(group => group !== current.name);
      const at = target.before === null ? next.length : next.indexOf(target.before);
      next.splice(at < 0 ? next.length : at, 0, current.name);
      onReorderGroups(next);
    }
  };

  useEffect(() => {
    window.addEventListener('pointerup', finish, true);
    window.addEventListener('pointercancel', finish, true);
    return () => {
      window.removeEventListener('pointerup', finish, true);
      window.removeEventListener('pointercancel', finish, true);
    };
  });

  const draggingConnectionId = drag?.kind === 'connection' ? drag.id : null;
  const draggingGroupName = drag?.kind === 'group' ? drag.name : null;

  const renderConnections = (group: string) => {
    const members = membersOf(group);
    if (!members.length) {
      return (
        <div className="rounded border border-dashed border-border px-2 py-2 text-center text-[11px] text-muted-foreground">
          {draggingConnectionId ? '拖到此处' : '暂无机器'}
        </div>
      );
    }
    return members.map(connection => {
      const beside = drop?.kind === 'beside' && drop.itemId === connection.id ? drop.edge : null;
      return (
        <div
          key={connection.id}
          data-tree-connection={connection.id}
          className={cn(
            'relative flex cursor-grab select-none items-center gap-2 rounded-md px-2 py-1.5 text-xs hover:bg-muted active:cursor-grabbing',
            draggingConnectionId === connection.id && 'opacity-40',
            beside === 'top' && 'before:absolute before:inset-x-1 before:-top-0.5 before:h-0.5 before:rounded-full before:bg-primary',
            beside === 'bottom' && 'after:absolute after:inset-x-1 after:-bottom-0.5 after:h-0.5 after:rounded-full after:bg-primary',
          )}
          onPointerDown={event => beginConnectionDrag(event, connection.id)}
        >
          <GripVertical className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
          <span className="min-w-0 flex-1 truncate font-medium" title={connection.name || connection.host}>
            {connection.name || connection.host}
          </span>
          <span className="shrink-0 truncate font-mono text-[10px] text-muted-foreground">
            {connection.username}@{connection.host}
          </span>
        </div>
      );
    });
  };

  const renderSection = (group: string) => {
    const isUngrouped = group === '';
    const isExpanded = expanded.has(group);
    const receiving = drop?.kind === 'intoGroup' && drop.group === group && draggingConnectionId !== null;
    const orderMark = drop?.kind === 'groupOrder' && drop.marker === group && draggingGroupName !== null;
    const count = membersOf(group).length;

    return (
      <section
        key={group || '__ungrouped__'}
        data-tree-group={group}
        className={cn(
          'relative rounded-md border transition-colors',
          receiving ? 'border-primary bg-primary/10' : 'border-border',
          draggingGroupName === group && 'opacity-45',
          orderMark && 'before:absolute before:inset-x-2 before:-top-1 before:h-0.5 before:rounded-full before:bg-primary',
        )}
      >
        <div className="flex items-center gap-1 px-2 py-1.5">
          {!isUngrouped ? (
            <button
              type="button"
              className="shrink-0 cursor-grab text-muted-foreground active:cursor-grabbing"
              aria-label={`拖拽排序：${group}`}
              onPointerDown={event => beginGroupDrag(event, group)}
            >
              <GripVertical className="h-4 w-4" />
            </button>
          ) : (
            <span className="w-4 shrink-0" />
          )}
          <button
            type="button"
            className="flex min-w-0 flex-1 cursor-pointer items-center gap-1 text-left"
            aria-expanded={isExpanded}
            onClick={() => toggle(group)}
          >
            {isExpanded ? <ChevronDown className="h-4 w-4 shrink-0 text-muted-foreground" /> : <ChevronRight className="h-4 w-4 shrink-0 text-muted-foreground" />}
            {isUngrouped && <span className="truncate text-sm text-muted-foreground">{UNGROUPED_LABEL}</span>}
          </button>
          {!isUngrouped && (
            <Input
              key={group}
              defaultValue={group}
              maxLength={40}
              aria-label={`分组名称：${group}`}
              className="h-7 min-w-0 flex-[3] border-transparent bg-transparent px-1 text-sm hover:border-input focus-visible:border-input"
              onBlur={event => onRenameGroup(group, event.target.value)}
            />
          )}
          <span className="shrink-0 px-1 text-xs tabular-nums text-muted-foreground">{count}</span>
          {!isUngrouped && (
            <Button
              type="button"
              variant="ghost"
              size="icon"
              className="h-7 w-7 shrink-0 text-destructive"
              title="删除分组（机器移到未分组）"
              onClick={() => onRemoveGroup(group)}
            >
              <Trash2 className="h-3.5 w-3.5" />
            </Button>
          )}
        </div>
        {isExpanded && <div className="grid gap-0.5 px-2 pb-2 pl-6">{renderConnections(group)}</div>}
      </section>
    );
  };

  return (
    <div ref={rootRef} className="grid max-h-80 select-none gap-2 overflow-y-auto pr-1" onPointerMove={handleMove}>
      {renderSection('')}
      {groups.map(renderSection)}
      {!connections.length && !groups.length && (
        <div className="p-3 text-center text-xs text-muted-foreground">还没有分组或机器。</div>
      )}
    </div>
  );
}
