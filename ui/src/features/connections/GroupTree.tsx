import { ChevronDown, ChevronRight, GripVertical, Trash2 } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { Input } from '../../components/ui/input';
import { Button } from '../../components/ui/button';
import { UNGROUPED_LABEL } from '../../lib/connection-groups';
import { cn } from '../../lib/utils';

export type TreeConnection = { id: string; name?: string; host?: string; username?: string };

type Props = {
  connections: TreeConnection[];
  groups: string[];
  /** connectionId → 分组名（空串代表未分组） */
  assignments: Map<string, string>;
  onAssign: (connectionId: string, group: string) => void;
  onReorderGroups: (next: string[]) => void;
  onRenameGroup: (oldName: string, newName: string) => void;
  onRemoveGroup: (name: string) => void;
};

// 拖拽落点用数据描述、由 React 渲染反馈。不要在运行时拼 Tailwind 类再 classList.add：
// Tailwind 编译期扫描源码，运行时才拼出的类名没有对应 CSS。
type DropTarget = { kind: 'group'; group: string } | { kind: 'groupOrder'; before: string | null };
type Pointer =
  | { kind: 'connection'; id: string; pointerId: number; startX: number; startY: number; active: boolean }
  | { kind: 'group'; name: string; pointerId: number; startY: number; active: boolean };

const DRAG_THRESHOLD = 6;

export function GroupTree({ connections, groups, assignments, onAssign, onReorderGroups, onRenameGroup, onRemoveGroup }: Props) {
  const rootRef = useRef<HTMLDivElement>(null);
  const pointerRef = useRef<Pointer | null>(null);
  const dropRef = useRef<DropTarget | null>(null);
  const [dragging, setDragging] = useState<Pointer | null>(null);
  const [drop, setDrop] = useState<DropTarget | null>(null);
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());

  const setDropTarget = (next: DropTarget | null) => {
    dropRef.current = next;
    setDrop(next);
  };

  const membersOf = (group: string) =>
    connections.filter(connection => (assignments.get(connection.id) || '') === group);

  const toggle = (group: string) =>
    setCollapsed(previous => {
      const next = new Set(previous);
      if (next.has(group)) next.delete(group);
      else next.add(group);
      return next;
    });

  // 命中判定：分组标题/分组区块 → 该分组；未分组区 → 移出分组。
  // 指针离开树时返回 null，松手即取消，不会把机器甩到别处。
  const computeDrop = (clientX: number, clientY: number, pointer: Pointer): DropTarget | null => {
    const root = rootRef.current;
    if (!root) return null;
    const element = document.elementFromPoint(clientX, clientY) as HTMLElement | null;
    if (!element || !root.contains(element)) return null;

    const section = element.closest<HTMLElement>('[data-tree-group]');
    if (pointer.kind === 'group') {
      // 分组排序：落在自己身上不算
      if (!section || section.dataset.treeGroup === pointer.name) return null;
      const name = section.dataset.treeGroup || '';
      if (!name) return null; // 未分组区块不参与排序
      const rect = section.getBoundingClientRect();
      const after = clientY >= rect.top + rect.height / 2;
      const index = groups.indexOf(name);
      return { kind: 'groupOrder', before: after ? groups[index + 1] ?? null : name };
    }
    return { kind: 'group', group: section?.dataset.treeGroup ?? '' };
  };

  const beginConnectionDrag = (event: React.PointerEvent, id: string) => {
    if (event.button !== 0) return;
    pointerRef.current = { kind: 'connection', id, pointerId: event.pointerId, startX: event.clientX, startY: event.clientY, active: false };
  };

  const beginGroupDrag = (event: React.PointerEvent, name: string) => {
    if (event.button !== 0) return;
    event.preventDefault();
    pointerRef.current = { kind: 'group', name, pointerId: event.pointerId, startY: event.clientY, active: false };
  };

  const handleMove = (event: React.PointerEvent) => {
    const pointer = pointerRef.current;
    if (!pointer || pointer.pointerId !== event.pointerId) return;
    // 阈值之下算点击，避免轻碰就进入拖拽
    const distance = pointer.kind === 'connection'
      ? Math.hypot(event.clientX - pointer.startX, event.clientY - pointer.startY)
      : Math.abs(event.clientY - pointer.startY);
    if (!pointer.active && distance < DRAG_THRESHOLD) return;
    if (!pointer.active) {
      pointer.active = true;
      setDragging(pointer);
      rootRef.current?.setPointerCapture?.(event.pointerId);
    }
    event.preventDefault();
    setDropTarget(computeDrop(event.clientX, event.clientY, pointer));
  };

  // 收尾挂 window：指针可能在树外松开，容器级监听收不到，拖拽状态会卡住。
  const finish = (event: PointerEvent) => {
    const pointer = pointerRef.current;
    if (!pointer || pointer.pointerId !== event.pointerId) return;
    const target = dropRef.current;
    pointerRef.current = null;
    setDragging(null);
    setDropTarget(null);
    if (!pointer.active || event.type === 'pointercancel' || !target) return;

    if (pointer.kind === 'connection' && target.kind === 'group') {
      onAssign(pointer.id, target.group);
      return;
    }
    if (pointer.kind === 'group' && target.kind === 'groupOrder') {
      const next = groups.filter(group => group !== pointer.name);
      const at = target.before === null ? next.length : next.indexOf(target.before);
      next.splice(at < 0 ? next.length : at, 0, pointer.name);
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

  const draggingConnection = dragging?.kind === 'connection' ? dragging.id : null;
  const draggingGroup = dragging?.kind === 'group' ? dragging.name : null;

  const renderConnections = (group: string) => {
    const members = membersOf(group);
    if (!members.length) {
      return (
        <div className="px-2 py-1.5 text-[11px] text-muted-foreground">
          {dragging?.kind === 'connection' ? '拖到此处' : '暂无机器'}
        </div>
      );
    }
    return members.map(connection => (
      <div
        key={connection.id}
        className={cn(
          'flex cursor-grab select-none items-center gap-2 rounded-md px-2 py-1.5 text-xs active:cursor-grabbing',
          'hover:bg-muted',
          draggingConnection === connection.id && 'opacity-40',
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
    ));
  };

  const renderSection = (group: string) => {
    const isUngrouped = group === '';
    const isCollapsed = collapsed.has(group);
    const receiving = drop?.kind === 'group' && drop.group === group && dragging?.kind === 'connection';
    const orderBefore = drop?.kind === 'groupOrder' && drop.before === group && draggingGroup !== null;
    const count = membersOf(group).length;

    return (
      <section
        key={group || '__ungrouped__'}
        data-tree-group={group}
        className={cn(
          'relative rounded-md border transition-colors',
          receiving ? 'border-primary bg-primary/10' : 'border-border',
          draggingGroup === group && 'opacity-45',
          orderBefore && 'before:absolute before:inset-x-2 before:-top-1 before:h-0.5 before:rounded-full before:bg-primary',
        )}
      >
        <div className="flex items-center gap-1 px-2 py-1.5">
          {!isUngrouped && (
            <button
              type="button"
              className="shrink-0 cursor-grab text-muted-foreground active:cursor-grabbing"
              aria-label={`拖拽排序：${group}`}
              onPointerDown={event => beginGroupDrag(event, group)}
            >
              <GripVertical className="h-4 w-4" />
            </button>
          )}
          <button
            type="button"
            className="shrink-0 cursor-pointer text-muted-foreground"
            aria-expanded={!isCollapsed}
            aria-label={isCollapsed ? '展开' : '收起'}
            onClick={() => toggle(group)}
          >
            {isCollapsed ? <ChevronRight className="h-4 w-4" /> : <ChevronDown className="h-4 w-4" />}
          </button>
          {isUngrouped ? (
            <span className="flex-1 text-sm text-muted-foreground">{UNGROUPED_LABEL}</span>
          ) : (
            <Input
              key={group}
              defaultValue={group}
              maxLength={40}
              aria-label={`分组名称：${group}`}
              className="h-7 flex-1 border-transparent bg-transparent px-1 text-sm hover:border-input focus-visible:border-input"
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
        {!isCollapsed && <div className="grid gap-0.5 px-2 pb-2 pl-6">{renderConnections(group)}</div>}
      </section>
    );
  };

  return (
    <div
      ref={rootRef}
      className="grid max-h-80 select-none gap-2 overflow-y-auto pr-1"
      onPointerMove={handleMove}
    >
      {renderSection('')}
      {groups.map(renderSection)}
      {!connections.length && !groups.length && (
        <div className="p-3 text-center text-xs text-muted-foreground">还没有分组或机器。</div>
      )}
    </div>
  );
}
