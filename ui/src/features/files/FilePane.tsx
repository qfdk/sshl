import type { MouseEvent, ReactNode } from 'react';
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
  onPathSubmit: () => void;
  onHome: () => void;
  onRefresh: () => void;
  onBrowse?: () => void;
  onOpen: (entry: FileEntry) => void;
  onRowSelect: (entry: FileEntry, index: number, event: MouseEvent<HTMLTableRowElement>) => void;
  onContextTarget: (entry: FileEntry | null) => void;
  onPermission?: (entry: FileEntry) => void;
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
  onHome,
  onRefresh,
  onBrowse,
  onOpen,
  onRowSelect,
  onContextTarget,
  onPermission,
}: FilePaneProps) {
  const isRemote = kind === 'remote';
  const rows: FileEntry[] = isRemote || !isRootPath(path, kind)
    ? (isRootPath(path, kind) ? files : [{ name: '..', path: '', isDirectory: true, isParent: true }, ...files])
    : files;

  return (
    <section className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden border border-border bg-background">
      <header className="flex items-center justify-between border-b border-border bg-muted/40 px-3 py-2 text-sm font-medium">
        <span>{isRemote ? '远程文件' : '本地文件'}</span>
        <div className="flex items-center gap-1">
          <Button type="button" variant="ghost" size="icon" className="h-7 w-7" title="主目录" disabled={disabled} onClick={onHome}>
            <House className="h-4 w-4" />
          </Button>
          <Button type="button" variant="ghost" size="icon" className="h-7 w-7" title="刷新" disabled={disabled} onClick={onRefresh}>
            <RefreshCw className="h-4 w-4" />
          </Button>
        </div>
      </header>

      <div className="flex gap-1 border-b border-border p-2">
        <Input
          value={path}
          disabled={disabled}
          className="h-8 min-w-0 text-xs"
          autoComplete="off"
          autoCapitalize="off"
          autoCorrect="off"
          spellCheck={false}
          onChange={event => onPathChange(event.target.value)}
          onKeyDown={event => {
            if (event.key === 'Enter') {
              event.preventDefault();
              onPathSubmit();
            }
          }}
        />
        <Button type="button" variant="outline" size="icon" className="h-8 w-8 shrink-0" title={isRemote ? '转到路径' : '选择目录'} disabled={disabled} onClick={onBrowse || onPathSubmit}>
          {onBrowse ? <FolderOpen className="h-4 w-4" /> : <ChevronRight className="h-4 w-4" />}
        </Button>
      </div>

      <div className="relative min-h-0 flex-1 overflow-auto select-none">
        <ContextMenu onOpenChange={open => { if (!open) onContextTarget(null); }}>
          <ContextMenuTrigger asChild>
            <div className="min-h-full" onContextMenu={event => { if (!(event.target as HTMLElement).closest('tr')) onContextTarget(null); }}>
              <Table className="table-fixed text-xs">
                <TableHeader className="sticky top-0 z-10 bg-muted/80 backdrop-blur">
                  <TableRow>
                    <TableHead className="h-9 px-3">名称</TableHead>
                    <TableHead className="h-9 w-20 px-3 text-right">大小</TableHead>
                    <TableHead className="h-9 w-36 px-3">修改日期</TableHead>
                    {isRemote && <TableHead className="h-9 w-24 px-3">所有者</TableHead>}
                    {isRemote && <TableHead className="h-9 w-16 px-3">权限</TableHead>}
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {emptyMessage && !loading && (
                    <TableRow>
                      <TableCell colSpan={isRemote ? 5 : 3} className="h-32 whitespace-pre-wrap text-center text-muted-foreground">
                        {emptyMessage}
                      </TableCell>
                    </TableRow>
                  )}
                  {!emptyMessage && !loading && rows.length === 0 && (
                    <TableRow>
                      <TableCell colSpan={isRemote ? 5 : 3} className="h-32 text-center text-muted-foreground">目录为空</TableCell>
                    </TableRow>
                  )}
                  {rows.map((entry, index) => {
                    const selected = selectedPaths.has(entry.path);
                    const parent = entry.name === '..';
                    return (
                      <TableRow
                        key={parent ? '..' : entry.path}
                        data-state={selected ? 'selected' : undefined}
                        className={cn('cursor-default', entry.isDirectory && 'cursor-pointer', selected && 'bg-accent')}
                        onMouseDown={event => event.preventDefault()}
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
                        <TableCell className={cn('truncate px-3 py-2', entry.isDirectory && 'text-primary')} title={entry.name}>
                          <span className="mr-2 inline-flex align-middle">
                            {entry.isDirectory ? <Folder className="h-4 w-4" /> : <FileText className="h-4 w-4" />}
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
        {loading && (
          <div className="absolute inset-0 flex items-center justify-center bg-background/70">
            <LoaderCircle className="h-6 w-6 animate-spin text-muted-foreground" />
          </div>
        )}
      </div>
    </section>
  );
}
