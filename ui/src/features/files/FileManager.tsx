import { useCallback, useEffect, useMemo, useState, useSyncExternalStore, type MouseEvent as ReactMouseEvent } from 'react';
import { Download, FolderPlus, RefreshCw, Trash2, Upload } from 'lucide-react';
import { getActiveTabId, getCurrentSessionId, subscribe } from '../../lib/app-state';
import { Button } from '../../components/ui/button';
import { Checkbox } from '../../components/ui/checkbox';
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '../../components/ui/dialog';
import { Input } from '../../components/ui/input';
import { Progress } from '../../components/ui/progress';
import { joinPath, parentPath, useFileList, type FileEntry, type FileKind } from './useFileList';
import { FilePane, type FileContextMenuItem } from './FilePane';
import { useTransfer } from './useTransfer';

const subscribeActiveTabId = (callback: () => void) => subscribe('activeTabId', callback);
const subscribeCurrentSessionId = (callback: () => void) => subscribe('currentSessionId', callback);

type AppWindow = Window & { api?: any; fileManager?: any; connectionManager?: any };
type PaneName = 'local' | 'remote';
type Selected = Record<PaneName, Set<string>>;

type PermissionTarget = {
  path: string;
  value: string;
};

function basename(path: string) {
  return path.split(/[\\/]/).pop() || path;
}

function parsePermissions(value: string) {
  if (value.length === 10) {
    return [1, 4, 7].map(index => (value[index] === 'r' ? 4 : 0) + (value[index + 1] === 'w' ? 2 : 0) + (value[index + 2] === 'x' ? 1 : 0)).join('');
  }
  return value;
}

function readablePermissions(octal: string) {
  if (!/^[0-7]{3}$/.test(octal)) return 'Invalid';
  return octal.split('').map(digit => {
    const value = Number(digit);
    return `${value & 4 ? 'r' : '-'}${value & 2 ? 'w' : '-'}${value & 1 ? 'x' : '-'}`;
  }).join('');
}

function permissionsToChecks(octal: string) {
  if (!/^[0-7]{3}$/.test(octal)) return [false, false, false, false, false, false, false, false, false];
  return octal.split('').flatMap(digit => {
    const value = Number(digit);
    return [Boolean(value & 4), Boolean(value & 2), Boolean(value & 1)];
  });
}

function checksToPermissions(checks: boolean[]) {
  return [0, 1, 2].map(group => (checks[group * 3] ? 4 : 0) + (checks[group * 3 + 1] ? 2 : 0) + (checks[group * 3 + 2] ? 1 : 0)).join('');
}

export function FileManager() {
  const activeTabId = useSyncExternalStore(subscribeActiveTabId, getActiveTabId, () => 'terminal');
  const sessionId = useSyncExternalStore(subscribeCurrentSessionId, getCurrentSessionId, () => null);
  const enabled = activeTabId === 'file-manager';
  const local = useFileList({ kind: 'local', sessionId: null, enabled });
  const remote = useFileList({ kind: 'remote', sessionId, enabled });
  const [selected, setSelected] = useState<Selected>({ local: new Set(), remote: new Set() });
  const [lastSelected, setLastSelected] = useState<{ pane: PaneName; index: number } | null>(null);
  const [contextTargets, setContextTargets] = useState<Record<PaneName, FileEntry | null>>({ local: null, remote: null });
  const [remoteEmptyConnectionId, setRemoteEmptyConnectionId] = useState<string | null>(null);
  const [permissionTarget, setPermissionTarget] = useState<PermissionTarget | null>(null);
  const [permissionValue, setPermissionValue] = useState('755');
  const [permissionChecks, setPermissionChecks] = useState(() => permissionsToChecks('755'));

  useEffect(() => {
    setSelected(current => ({ ...current, local: new Set(), remote: new Set() }));
    setLastSelected(null);
  }, [local.path, remote.path]);

  const clearSelection = useCallback((pane: PaneName) => {
    setSelected(current => ({ ...current, [pane]: new Set() }));
  }, []);

  const selectRow = useCallback((pane: PaneName, entry: FileEntry, index: number, event: ReactMouseEvent<HTMLTableRowElement>) => {
    setSelected(current => {
      const next = new Set<string>();
      const otherPane: PaneName = pane === 'local' ? 'remote' : 'local';
      if (event.metaKey || event.ctrlKey) {
        current[pane].forEach(path => next.add(path));
        if (next.has(entry.path)) next.delete(entry.path);
        else next.add(entry.path);
      } else if (event.shiftKey && lastSelected?.pane === pane) {
        const entries = pane === 'local' ? local.files : remote.files;
        const first = Math.min(lastSelected.index, index);
        const last = Math.max(lastSelected.index, index);
        entries.slice(first, last + 1).forEach(item => next.add(item.path));
      } else {
        next.add(entry.path);
      }
      return { ...current, [pane]: next, [otherPane]: new Set() };
    });
    setLastSelected({ pane, index });
  }, [lastSelected, local.files, remote.files]);

  const ensureContextSelection = useCallback((pane: PaneName, entry: FileEntry | null) => {
    setContextTargets(current => ({ ...current, [pane]: entry }));
    if (!entry) return;
    setSelected(current => current[pane].has(entry.path)
      ? current
      : { ...current, [pane]: new Set([entry.path]), [pane === 'local' ? 'remote' : 'local']: new Set() });
  }, []);

  const refreshLocal = useCallback(() => { void local.load(local.path, true); }, [local]);
  const refreshRemote = useCallback(() => { if (sessionId) void remote.load(remote.path || '/', true); }, [remote, sessionId]);

  const onUploadComplete = useCallback(() => {
    if (sessionId) {
      remote.invalidate(remote.path, sessionId);
      void remote.load(remote.path || '/', true);
    }
  }, [remote, sessionId]);
  const onDownloadComplete = useCallback(() => {
    local.invalidate(local.path);
    void local.load(local.path, true);
  }, [local]);
  const transfer = useTransfer(sessionId, onUploadComplete, onDownloadComplete);

  const uploadItems = useCallback(async (items: FileEntry[]) => {
    if (!sessionId) return;
    for (const item of items) {
      const destination = remote.path === '/' ? `/${basename(item.path)}` : `${remote.path}/${basename(item.path)}`;
      if (item.isDirectory) await transfer.uploadDirectory(item.path, destination);
      else await transfer.uploadFile(item.path, destination);
    }
  }, [remote.path, sessionId, transfer]);

  const downloadItems = useCallback(async (items: FileEntry[]) => {
    if (!sessionId || !local.path) return;
    for (const item of items) {
      if (item.isDirectory) await transfer.downloadDirectory(item.path, joinPath(local.path, basename(item.path)));
      else await transfer.downloadFile(item.path, joinPath(local.path, basename(item.path)));
    }
  }, [local.path, sessionId, transfer]);

  const deleteItems = useCallback(async (pane: PaneName, items: FileEntry[]) => {
    if (!items.length) return;
    const names = items.map(item => basename(item.path)).join(', ');
    const message = items.length > 1
      ? `确定要删除 ${items.length} 个项目吗？\n${names}\n此操作不可恢复！`
      : `确定要删除${items[0].isDirectory ? '目录' : '文件'} "${basename(items[0].path)}"${items[0].isDirectory ? ' 及其所有内容' : ''}吗？此操作不可恢复！`;
    if (!await (window as AppWindow).api.dialog.confirm(message, items.length > 1 ? '批量删除' : '删除')) return;

    for (const item of items) {
      const result = pane === 'remote'
        ? await (window as AppWindow).api.ssh.execute(sessionId, `${item.isDirectory ? 'rm -rf' : 'rm -f'} "${item.path}"`)
        : await (window as AppWindow).api.file[item.isDirectory ? 'deleteLocalDirectory' : 'deleteLocal'](item.path);
      if (!result?.success) {
        window.alert(`删除失败: ${result?.error || '未知错误'}`);
        continue;
      }
      if (pane === 'remote') remote.invalidate(remote.path, sessionId);
      else local.invalidate(local.path);
    }
    if (pane === 'remote') void remote.load(remote.path || '/', true);
    else void local.load(local.path, true);
  }, [local, remote, sessionId]);

  const createRemoteDirectory = useCallback(async () => {
    if (!sessionId) return;
    const name = window.prompt('请输入文件夹名称');
    if (!name) return;
    if (name.includes('/') || name.includes('\\')) {
      window.alert('文件夹名称不能包含斜杠');
      return;
    }
    const fullPath = remote.path === '/' ? `/${name}` : `${remote.path}/${name}`;
    const result = await (window as AppWindow).api.file.createRemoteDirectory(sessionId, fullPath);
    if (result?.success) {
      remote.invalidate(remote.path, sessionId);
      void remote.load(remote.path || '/', true);
    } else {
      window.alert(`创建文件夹失败: ${result?.error || '未知错误'}`);
    }
  }, [remote, sessionId]);

  const remoteItems = useMemo(() => remote.files.filter(item => selected.remote.has(item.path)), [remote.files, selected.remote]);
  const localItems = useMemo(() => local.files.filter(item => selected.local.has(item.path)), [local.files, selected.local]);

  const menuItems = useCallback((pane: PaneName): FileContextMenuItem[] => {
    const target = contextTargets[pane];
    const items = pane === 'remote' ? remoteItems : localItems;
    const selectedItems = target && !items.some(item => item.path === target.path) ? [target] : items;
    const actions: FileContextMenuItem[] = [{ label: '刷新', icon: <RefreshCw className="mr-2 h-4 w-4" />, onSelect: pane === 'remote' ? refreshRemote : refreshLocal }];

    if (!target) {
      if (pane === 'remote') actions.unshift({ label: '新建文件夹', icon: <FolderPlus className="mr-2 h-4 w-4" />, onSelect: createRemoteDirectory, disabled: !sessionId });
      else actions.unshift({ label: '选择文件夹上传', icon: <Upload className="mr-2 h-4 w-4" />, onSelect: async () => {
        const result = await (window as AppWindow).api.dialog.selectDirectory();
        if (!result?.canceled && result.filePaths?.[0] && sessionId) {
          const name = basename(result.filePaths[0]);
          await transfer.uploadDirectory(result.filePaths[0], remote.path === '/' ? `/${name}` : `${remote.path}/${name}`);
        }
      }, disabled: !sessionId });
      return actions;
    }

    if (selectedItems.length > 1) {
      actions.unshift(
        ...(pane === 'remote'
          ? [{ label: `下载 ${selectedItems.length} 个项目`, icon: <Download className="mr-2 h-4 w-4" />, onSelect: () => void downloadItems(selectedItems) }]
          : [{ label: `上传 ${selectedItems.length} 个项目`, icon: <Upload className="mr-2 h-4 w-4" />, onSelect: () => void uploadItems(selectedItems) }]),
        { label: `删除 ${selectedItems.length} 个项目`, icon: <Trash2 className="mr-2 h-4 w-4" />, onSelect: () => void deleteItems(pane, selectedItems) },
      );
    } else {
      const item = selectedItems[0];
      if (pane === 'remote') {
        actions.unshift({ label: item.isDirectory ? '下载文件夹' : '下载文件', icon: <Download className="mr-2 h-4 w-4" />, onSelect: () => void downloadItems([item]) });
      } else {
        const destination = remote.path === '/' ? `/${basename(item.path)}` : `${remote.path}/${basename(item.path)}`;
        actions.unshift({ label: item.isDirectory ? '上传文件夹' : '上传文件', icon: <Upload className="mr-2 h-4 w-4" />, onSelect: () => void (item.isDirectory ? transfer.uploadDirectory(item.path, destination) : transfer.uploadFile(item.path, destination)) });
      }
      actions.push({ label: item.isDirectory ? '删除目录' : '删除文件', icon: <Trash2 className="mr-2 h-4 w-4" />, onSelect: () => void deleteItems(pane, [item]) });
    }
    return actions;
  }, [contextTargets, createRemoteDirectory, deleteItems, downloadItems, localItems, refreshLocal, refreshRemote, remote.path, remoteItems, sessionId, transfer, uploadItems]);

  const openEntry = useCallback((kind: FileKind, entry: FileEntry) => {
    const list = kind === 'local' ? local : remote;
    void list.load(entry.name === '..' ? parentPath(list.path, kind) : entry.path, true);
  }, [local, remote]);

  const openPermissions = useCallback((entry: FileEntry) => {
    const value = entry.permissions ? entry.permissions.toString(8).slice(-3) : '-';
    setPermissionTarget({ path: entry.path, value });
    const octal = parsePermissions(value);
    setPermissionValue(octal);
    setPermissionChecks(permissionsToChecks(octal));
  }, []);

  const applyPermissions = useCallback(async () => {
    if (!permissionTarget || !sessionId || !/^[0-7]{3}$/.test(permissionValue)) return;
    const result = await (window as AppWindow).api.file.changePermissions(sessionId, permissionTarget.path, permissionValue);
    if (result?.success) {
      setPermissionTarget(null);
      remote.invalidate(remote.path, sessionId);
      void remote.load(remote.path || '/', true);
    } else {
      window.alert(`权限修改失败: ${result?.error || '未知错误'}`);
    }
  }, [permissionTarget, permissionValue, remote, sessionId]);

  useEffect(() => {
    const appWindow = window as AppWindow;
    const bridge = {
      get activeTransfers() { return transfer.activeTransfers; },
      clearCache: () => { setRemoteEmptyConnectionId(null); remote.clearCache(); remote.setEmptyMessage(null); clearSelection('remote'); },
      // useTerminal.ts is intentionally left unchanged; these aliases keep its disconnect guard thin.
      clearFileManagerCache: () => { setRemoteEmptyConnectionId(null); remote.clearCache(); remote.setEmptyMessage(null); clearSelection('remote'); },
      refresh: () => { if (sessionId) void remote.load(remote.path || '/', true); },
      showRemoteEmptyState: (message: string, connectionId?: string | null) => { setRemoteEmptyConnectionId(connectionId || null); remote.setEmptyMessage(message); remote.setPath(''); remote.clearCache(); clearSelection('remote'); },
      renderRemoteEmptyState: (message: string, connectionId?: string | null) => { setRemoteEmptyConnectionId(connectionId || null); remote.setEmptyMessage(message); remote.setPath(''); remote.clearCache(); clearSelection('remote'); },
    };
    appWindow.fileManager = bridge;
    return () => {
      if (appWindow.fileManager === bridge) delete appWindow.fileManager;
    };
  }, [clearSelection, remote, sessionId, transfer.activeTransfers]);

  useEffect(() => {
    if (sessionId) setRemoteEmptyConnectionId(null);
  }, [sessionId]);

  useEffect(() => {
    const divider = document.getElementById('pane-divider');
    const split = divider?.parentElement;
    const localPane = split?.querySelector<HTMLElement>('[data-file-pane="local"]');
    const remotePane = split?.querySelector<HTMLElement>('[data-file-pane="remote"]');
    if (!divider || !split || !localPane || !remotePane) return;
    const saved = Number(localStorage.getItem('sshl.localPaneFlex'));
    if (saved > 0) localPane.style.flexGrow = String(saved);
    let startX = 0;
    let startLocal = 0;
    let startRemote = 0;
    let totalFlex = 2;
    const move = (event: MouseEvent) => {
      const width = Math.max(220, Math.min(startLocal + event.clientX - startX, startLocal + startRemote - 220));
      const ratio = width / (startLocal + startRemote);
      localPane.style.flexGrow = String(ratio * totalFlex);
      remotePane.style.flexGrow = String(totalFlex - ratio * totalFlex);
    };
    const up = () => {
      document.removeEventListener('mousemove', move);
      document.removeEventListener('mouseup', up);
      localStorage.setItem('sshl.localPaneFlex', localPane.style.flexGrow || '1');
    };
    const down = (event: globalThis.MouseEvent) => {
      event.preventDefault();
      const localRect = localPane.getBoundingClientRect();
      const remoteRect = remotePane.getBoundingClientRect();
      startX = event.clientX;
      startLocal = localRect.width;
      startRemote = remoteRect.width;
      totalFlex = (Number(localPane.style.flexGrow) || 1) + (Number(remotePane.style.flexGrow) || 1);
      document.addEventListener('mousemove', move);
      document.addEventListener('mouseup', up);
    };
    divider.addEventListener('mousedown', down);
    return () => {
      divider.removeEventListener('mousedown', down);
      document.removeEventListener('mousemove', move);
      document.removeEventListener('mouseup', up);
    };
  }, []);

  const permissionGroupLabels = ['所有者', '组', '其他用户'];
  return (
    <div className="relative flex h-full min-h-0 flex-col" data-file-manager>
      <div className="flex min-h-0 flex-1 overflow-hidden" id="file-manager-split-pane">
        <div className="flex min-h-0 min-w-0 flex-1" data-file-pane="local">
          <FilePane
            kind="local"
            path={local.path}
            files={local.files}
            loading={local.loading}
            emptyMessage={local.error}
            selectedPaths={selected.local}
            contextMenuItems={menuItems('local')}
            onPathChange={local.setPath}
            onPathSubmit={() => void local.load(local.path, true)}
            onHome={async () => { const home = await (window as AppWindow).api.file.getHomeDir(); if (home) void local.load(home, true); }}
            onRefresh={refreshLocal}
            onBrowse={() => void local.browse()}
            onOpen={entry => openEntry('local', entry)}
            onRowSelect={(entry, index, event) => selectRow('local', entry, index, event)}
            onContextTarget={entry => ensureContextSelection('local', entry)}
          />
        </div>
        <div id="pane-divider" className="w-1 shrink-0 cursor-col-resize bg-border" title="拖动调整面板宽度" />
        <div className="flex min-h-0 min-w-0 flex-1" data-file-pane="remote">
          <FilePane
            kind="remote"
            path={remote.path}
            files={remote.files}
            loading={remote.loading}
            disabled={!sessionId}
            emptyMessage={remoteEmptyConnectionId ? (
              <div className="flex flex-col items-center gap-3">
                <span>{remote.emptyMessage}</span>
                <Button type="button" size="sm" onClick={() => (window as AppWindow).connectionManager?.connectToSaved?.(remoteEmptyConnectionId)}>
                  重新连接
                </Button>
              </div>
            ) : (remote.emptyMessage || (!sessionId ? '请先连接到服务器' : remote.error))}
            selectedPaths={selected.remote}
            contextMenuItems={menuItems('remote')}
            onPathChange={remote.setPath}
            onPathSubmit={() => void remote.load(remote.path, true)}
            onHome={async () => {
              if (!sessionId) return;
              const result = await (window as AppWindow).api.ssh.execute(sessionId, 'echo "$HOME"');
              const home = (result?.output ?? result?.data ?? result)?.toString().trim();
              if (home) void remote.load(home, true);
            }}
            onRefresh={refreshRemote}
            onOpen={entry => openEntry('remote', entry)}
            onRowSelect={(entry, index, event) => selectRow('remote', entry, index, event)}
            onContextTarget={entry => ensureContextSelection('remote', entry)}
            onPermission={openPermissions}
          />
        </div>
      </div>

      {transfer.status.active && (
        <div className="flex h-8 shrink-0 items-center gap-3 border-t border-border bg-muted/40 px-3 text-xs">
          <Progress value={transfer.status.progress} className="h-1.5 w-48" />
          <span className="min-w-0 flex-1 truncate">{transfer.status.info}</span>
        </div>
      )}

      <Dialog open={Boolean(permissionTarget)} onOpenChange={open => { if (!open) setPermissionTarget(null); }}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>修改文件权限</DialogTitle>
            <DialogDescription className="truncate">{permissionTarget?.path}</DialogDescription>
          </DialogHeader>
          <div className="space-y-4">
            <div className="flex items-center gap-3">
              <label htmlFor="new-permissions" className="text-sm font-medium">八进制权限</label>
              <Input id="new-permissions" value={permissionValue} maxLength={3} className="w-20 font-mono" onChange={event => {
                const value = event.target.value;
                setPermissionValue(value);
                setPermissionChecks(permissionsToChecks(value));
              }} />
              <span className="font-mono text-sm text-muted-foreground">= {readablePermissions(permissionValue)}</span>
            </div>
            <div className="flex flex-wrap gap-2">
              <span className="mr-2 text-sm text-muted-foreground">常用权限:</span>
              {['755', '644', '777', '600'].map(value => <Button key={value} type="button" variant="outline" size="sm" onClick={() => { setPermissionValue(value); setPermissionChecks(permissionsToChecks(value)); }}>{value}</Button>)}
            </div>
            {permissionGroupLabels.map((label, group) => (
              <div key={label} className="space-y-2 border-t border-border pt-3">
                <div className="text-sm font-medium">{label}</div>
                <div className="flex gap-4">
                  {['读取', '写入', '执行'].map((name, bit) => {
                    const index = group * 3 + bit;
                    return <label key={name} className="flex items-center gap-2 text-sm"><Checkbox checked={permissionChecks[index]} onCheckedChange={checked => {
                      const next = [...permissionChecks];
                      next[index] = checked === true;
                      setPermissionChecks(next);
                      setPermissionValue(checksToPermissions(next));
                    }} /><span>{name}</span></label>;
                  })}
                </div>
              </div>
            ))}
          </div>
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => setPermissionTarget(null)}>取消</Button>
            <Button type="button" disabled={!/^[0-7]{3}$/.test(permissionValue)} onClick={() => void applyPermissions()}>应用</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
