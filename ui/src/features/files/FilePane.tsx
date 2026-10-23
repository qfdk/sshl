import { useEffect, useId, useLayoutEffect, useRef, useState, type MouseEvent, type ReactNode, type UIEvent } from 'react';
import { ChevronRight, FileText, Folder, FolderOpen, House, LoaderCircle, RefreshCw } from 'lucide-react';
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuTrigger,
} from '../../components/ui/context-menu';
import { Button } from '../../components/ui/button';
import { Input } from '../../components/ui/input';
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '../../components/ui/table';
import { cn } from '../../lib/utils';
import { formatDate, formatFileSize, formatPermissions, type FileEntry, type FileKind, isRootPath } from './useFileList';
import { matchDirectories, splitCompletionPath, tabCompletion } from './path-completion';
import { isSelectAllShortcut } from './shortcuts';
import { visibleRowWindow } from './virtual-rows';

// 不超过这个行数就全量渲染：几屏以内的行渲染只需几毫秒，虚拟化没有可感知的收益，
// 反而引入 spacer 与滚动区间的边界情况。/usr/bin、日志目录这类上千条目才是要解决的。
const VIRTUALIZE_MIN_ROWS = 200;
// 可视区上下各多渲染的行数，快速滚动时不露白。
const OVERSCAN_ROWS = 10;
// 行高固定为整数像素：内容自然高度约 33.84px（py-2 + 图标撑开的行框 + border-b），
// 表格布局会把紧跟小数高度 spacer 的行贴到像素网格上，实测行高在 33.34/33.84 间跳，
// 测量第一行得到的值不可靠。显式 34px 让 spacer 与行高都是整数，几何关系精确。
const ROW_HEIGHT = 34;

export type FileContextMenuItem = {
  label: string;
  icon?: ReactNode;
  onSelect: () => void;
  disabled?: boolean;
};

export type FilePaneProps = {
  kind: FileKind;
  path: string;
  files: FileEntry[];
  loading: boolean;
  disabled?: boolean;
  emptyMessage?: ReactNode;
  selectedPaths: Set<string>;
  contextMenuItems: FileContextMenuItem[];
  onPathChange: (path: string) => void;
  onPathSubmit: (path?: string) => void;
  onListDirectory?: (directory: string) => Promise<FileEntry[] | null>;
  onHome: () => void;
  onRefresh: () => void;
  onBrowse?: () => void;
  onOpen: (entry: FileEntry) => void;
  onRowSelect: (entry: FileEntry, index: number, event: MouseEvent<HTMLTableRowElement>) => void;
  onContextTarget: (entry: FileEntry | null) => void;
  onPermission?: (entry: FileEntry) => void;
  onSelectAll?: () => void;
};

export function FilePane({
  kind,
  path,
  files,
  loading,
  disabled = false,
  emptyMessage,
  selectedPaths,
  contextMenuItems,
  onPathChange,
  onPathSubmit,
  onListDirectory,
  onHome,
  onRefresh,
  onBrowse,
  onOpen,
  onRowSelect,
  onContextTarget,
  onPermission,
  onSelectAll,
}: FilePaneProps) {
  const suggestionListId = useId();
  const suggestionListRef = useRef<HTMLDivElement>(null);
  // 补全只在用户在地址栏里敲字后出现；双击进目录这类从外部改路径不弹。
  const [suggesting, setSuggesting] = useState(false);
  const [highlight, setHighlight] = useState(-1);
  const [listing, setListing] = useState<{ directory: string; entries: FileEntry[] } | null>(null);
  const query = suggesting && !disabled && onListDirectory ? splitCompletionPath(path, kind) : null;
  const completionDirectory = query?.directory ?? null;
  const suggestions = query && listing?.directory === query.directory ? matchDirectories(listing.entries, query.prefix) : [];

  // 只在目录部分变了（敲了分隔符或 Tab 补全）才取列表；同一层里继续输入只在本地过滤。
  useEffect(() => {
    if (!completionDirectory || !onListDirectory) return;
    let current = true;
    void onListDirectory(completionDirectory).then(entries => {
      if (current && entries) setListing({ directory: completionDirectory, entries });
    });
    return () => { current = false; };
  }, [completionDirectory, onListDirectory]);

  useEffect(() => {
    if (highlight >= 0) suggestionListRef.current?.children[highlight]?.scrollIntoView({ block: 'nearest' });
  }, [highlight]);

  const openSuggestion = (entry: FileEntry) => {
    if (!query) return;
    setSuggesting(false);
    onPathSubmit(`${query.base}${entry.name}`);
  };

  const isRemote = kind === 'remote';
  const rows: FileEntry[] = isRemote || !isRootPath(path, kind)
    ? (isRootPath(path, kind) ? files : [{ name: '..', path: '', isDirectory: true, isParent: true }, ...files])
    : files;
  const columnCount = isRemote ? 5 : 3;

  const scrollRef = useRef<HTMLDivElement>(null);
  const headerRef = useRef<HTMLTableSectionElement>(null);
  const listedPathRef = useRef(path);
  const [scrollTop, setScrollTop] = useState(0);
  const [viewportHeight, setViewportHeight] = useState(0);
  const [headerHeight, setHeaderHeight] = useState(0);
  const virtual = rows.length > VIRTUALIZE_MIN_ROWS;
  // 量化到行边界：同一行内的滚动不改变渲染区间，React 对相同的 state 直接跳过重渲。
  const quantize = (value: number) => Math.floor(value / ROW_HEIGHT) * ROW_HEIGHT;

  useEffect(() => {
    const element = scrollRef.current;
    if (!element) return;
    // 文件管理标签页隐藏时视口为 0，切回来由 ResizeObserver 补上真实高度。
    const measure = () => {
      setViewportHeight(element.clientHeight);
      setHeaderHeight(headerRef.current?.getBoundingClientRect().height ?? 0);
    };
    measure();
    if (typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  // 不跟踪滚动的小列表刷新成大列表时，从真实 scrollTop 起算区间，不露白。
  useLayoutEffect(() => {
    if (virtual && scrollRef.current) setScrollTop(quantize(scrollRef.current.scrollTop));
  }, [virtual]);

  // 进入新目录时回到顶部；同目录刷新保留滚动位置。不能直接依赖 path：
  // 地址栏输入时 path 每敲一个键都会变。
  useEffect(() => {
    if (listedPathRef.current === path) return;
    listedPathRef.current = path;
    if (scrollRef.current) scrollRef.current.scrollTop = 0;
    setScrollTop(0);
  }, [files]);

  const onScroll = (event: UIEvent<HTMLDivElement>) => {
    if (virtual) setScrollTop(quantize(event.currentTarget.scrollTop));
  };

  const range = virtual
    ? visibleRowWindow(rows.length, ROW_HEIGHT, scrollTop - headerHeight, viewportHeight, OVERSCAN_ROWS)
    : { start: 0, end: rows.length, padTop: 0, padBottom: 0 };

  return (
    <section className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden rounded-[14px] border border-border bg-card shadow-[0_1px_2px_rgba(22,24,29,0.06),0_8px_24px_rgba(22,24,29,0.06)]">
      <header className="flex items-center gap-1.5 px-3.5 pb-2.5 pt-3">
        <span className="mr-1 shrink-0 text-sm font-semibold">{isRemote ? '远程文件' : '本地文件'}</span>
        <div className="relative min-w-0 flex-1">
          <Input
            value={path}
            disabled={disabled}
            className="h-8 rounded-[9px] bg-muted font-mono text-xs"
            role="combobox"
            aria-expanded={suggestions.length > 0}
            aria-controls={suggestionListId}
            aria-autocomplete="list"
            aria-activedescendant={suggestions[highlight] ? `${suggestionListId}-${highlight}` : undefined}
            autoComplete="off"
            autoCapitalize="off"
            autoCorrect="off"
            spellCheck={false}
            onChange={event => {
              onPathChange(event.target.value);
              setSuggesting(true);
              setHighlight(-1);
            }}
            onBlur={() => setSuggesting(false)}
            onKeyDown={event => {
              // 输入法选词时的方向键、回车属于输入法
              if (event.nativeEvent.isComposing) return;
              if (event.key === 'ArrowDown' && !suggesting) {
                event.preventDefault();
                setSuggesting(true);
              } else if ((event.key === 'ArrowDown' || event.key === 'ArrowUp') && suggestions.length) {
                event.preventDefault();
                const last = suggestions.length - 1;
                setHighlight(index => event.key === 'ArrowDown' ? (index >= last ? 0 : index + 1) : (index <= 0 ? last : index - 1));
              } else if (event.key === 'Tab' && query && suggestions.length) {
                event.preventDefault();
                const completed = tabCompletion(query, suggestions, highlight);
                if (completed) {
                  onPathChange(completed);
                  setHighlight(-1);
                }
              } else if (event.key === 'Escape' && suggestions.length) {
                event.preventDefault();
                setSuggesting(false);
              } else if (event.key === 'Enter') {
                event.preventDefault();
                if (suggestions[highlight]) {
                  openSuggestion(suggestions[highlight]);
                } else {
                  setSuggesting(false);
                  onPathSubmit();
                }
              }
            }}
          />
          {suggestions.length > 0 && (
            <div ref={suggestionListRef} id={suggestionListId} role="listbox" aria-label="子目录" className="absolute inset-x-0 top-full z-20 mt-1 max-h-64 overflow-auto rounded-[10px] border border-border bg-popover p-1 text-popover-foreground shadow-md">
              {suggestions.map((entry, index) => (
                <div
                  key={entry.name}
                  id={`${suggestionListId}-${index}`}
                  role="option"
                  aria-selected={index === highlight}
                  className={cn('flex cursor-pointer items-center gap-2 rounded-md px-2 py-1.5 font-mono text-xs', index === highlight && 'bg-accent text-accent-foreground')}
                  // 不让输入框失焦，否则 blur 先把下拉收掉，click 落空
                  onMouseDown={event => event.preventDefault()}
                  onMouseEnter={() => setHighlight(index)}
                  onClick={() => openSuggestion(entry)}
                >
                  <Folder className="h-3.5 w-3.5 shrink-0 text-primary" />
                  <span className="truncate">{entry.name}</span>
                </div>
              ))}
            </div>
          )}
        </div>
        <Button type="button" variant="ghost" size="icon" className="h-8 w-8 shrink-0" title={isRemote ? '转到路径' : '选择目录'} disabled={disabled} onClick={onBrowse || (() => onPathSubmit())}>
          {onBrowse ? <FolderOpen className="h-4 w-4" /> : <ChevronRight className="h-4 w-4" />}
        </Button>
        <Button type="button" variant="ghost" size="icon" className="h-8 w-8 shrink-0" title="主目录" disabled={disabled} onClick={onHome}>
          <House className="h-4 w-4" />
        </Button>
        <Button type="button" variant="ghost" size="icon" className="h-8 w-8 shrink-0" title="刷新" disabled={disabled} onClick={onRefresh}>
          <RefreshCw className="h-4 w-4" />
        </Button>
      </header>

      {/* 遮罩不能放进滚动容器：absolute 会跟着内容滚走，滚下去后刷新只盖住顶部一屏。
          isolate 把遮罩的 z-20 限定在列表内，压住 sticky 表头，又不盖地址栏补全下拉。 */}
      <div className="relative isolate flex min-h-0 flex-1 flex-col">
        {/* 列表可获得焦点，Cmd+A 只作用于用户点过的这一侧；地址栏在列表外，Cmd+A 仍是全选文字。 */}
        <div
          ref={scrollRef}
          tabIndex={-1}
          className="min-h-0 flex-1 overflow-auto outline-none select-none"
          onScroll={onScroll}
          onKeyDown={event => {
            if (!isSelectAllShortcut(event)) return;
            event.preventDefault();
            onSelectAll?.();
          }}
        >
          <ContextMenu onOpenChange={open => { if (!open) onContextTarget(null); }}>
            <ContextMenuTrigger asChild>
              <div className="min-h-full" onContextMenu={event => { if (!(event.target as HTMLElement).closest('tr')) onContextTarget(null); }}>
                <Table className="table-fixed text-xs">
                  {/* border-collapse 下行边框不随 sticky 走，分隔线用单元格的 inset 阴影画（WebKit 不画 thead 上的阴影） */}
                  <TableHeader ref={headerRef} className="sticky top-0 z-10 bg-card [&_th]:shadow-[inset_0_-1px_0_var(--color-border)] [&_tr]:border-b-0">
                    <TableRow>
                      <TableHead className="h-8 px-3.5 font-medium">名称</TableHead>
                      <TableHead className="h-8 w-20 px-3 text-right font-medium">大小</TableHead>
                      <TableHead className="h-8 w-36 px-3 font-medium">修改日期</TableHead>
                      {isRemote && <TableHead className="h-8 w-24 px-3 font-medium">所有者</TableHead>}
                      {isRemote && <TableHead className="h-8 w-16 px-3 font-medium">权限</TableHead>}
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {emptyMessage && !loading && (
                      <TableRow>
                        <TableCell colSpan={columnCount} className="h-32 whitespace-pre-wrap text-center text-muted-foreground">
                          {emptyMessage}
                        </TableCell>
                      </TableRow>
                    )}
                    {!emptyMessage && !loading && rows.length === 0 && (
                      <TableRow>
                        <TableCell colSpan={columnCount} className="h-32 text-center text-muted-foreground">目录为空</TableCell>
                      </TableRow>
                    )}
                    {/* spacer 只在高度 > 0 时渲染：滚到底时最后一条数据行仍是 last-child，沿用 TableBody 去掉末行边框的样式。 */}
                    {range.padTop > 0 && (
                      <tr aria-hidden="true"><td colSpan={columnCount} className="p-0" style={{ height: range.padTop }} /></tr>
                    )}
                    {rows.slice(range.start, range.end).map(entry => {
                      const selected = selectedPaths.has(entry.path);
                      const parent = entry.name === '..';
                      return (
                        <TableRow
                          key={parent ? '..' : entry.path}
                          data-state={selected ? 'selected' : undefined}
                          style={{ height: ROW_HEIGHT }}
                          className={cn('cursor-default', entry.isDirectory && 'cursor-pointer', selected && 'bg-accent text-accent-foreground hover:bg-accent')}
                          // 阻止默认行为避免拖选文字，但焦点要交给列表，否则会留在终端里、收不到 Cmd+A。
                          onMouseDown={event => {
                            event.preventDefault();
                            scrollRef.current?.focus({ preventScroll: true });
                          }}
                          onClick={event => {
                            if (!parent) onRowSelect(entry, files.indexOf(entry), event);
                          }}
                          onDoubleClick={() => {
                            if (entry.isDirectory) onOpen(entry);
                          }}
                          onContextMenu={() => {
                            if (!parent) onContextTarget(entry);
                          }}
                        >
                          <TableCell className={cn('truncate px-3.5 py-2', selected && 'font-medium')} title={entry.name}>
                            <span className="mr-2 inline-flex align-middle">
                              {entry.isDirectory ? <Folder className="h-4 w-4 text-primary" /> : <FileText className="h-4 w-4 text-muted-foreground" />}
                            </span>
                            {entry.name}
                          </TableCell>
                          <TableCell className="whitespace-nowrap px-3 py-2 text-right tabular-nums text-muted-foreground">{entry.isDirectory ? '-' : formatFileSize(entry.size || 0)}</TableCell>
                          <TableCell className="whitespace-nowrap px-3 py-2 tabular-nums text-muted-foreground">{formatDate(entry.modifyTime)}</TableCell>
                          {isRemote && <TableCell className="truncate px-3 py-2 text-muted-foreground">{entry.owner || 'unknown'}</TableCell>}
                          {isRemote && (
                            <TableCell
                              className="cursor-pointer px-3 py-2"
                              title="点击修改权限"
                              onClick={event => {
                                event.stopPropagation();
                                if (!parent) onPermission?.(entry);
                              }}
                            >
                              {formatPermissions(entry.permissions)}
                            </TableCell>
                          )}
                        </TableRow>
                      );
                    })}
                    {range.padBottom > 0 && (
                      <tr aria-hidden="true"><td colSpan={columnCount} className="p-0" style={{ height: range.padBottom }} /></tr>
                    )}
                  </TableBody>
                </Table>
              </div>
            </ContextMenuTrigger>
            <ContextMenuContent className="min-w-48">
              {contextMenuItems.map(item => (
                <ContextMenuItem key={item.label} disabled={item.disabled} onSelect={item.onSelect}>
                  {item.icon}
                  <span>{item.label}</span>
                </ContextMenuItem>
              ))}
            </ContextMenuContent>
          </ContextMenu>
        </div>
        {loading && (
          <div className="absolute inset-0 z-20 flex items-center justify-center bg-background/70">
            <LoaderCircle className="h-6 w-6 animate-spin text-muted-foreground" />
          </div>
        )}
      </div>
    </section>
  );
}
