import { memo, useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore, type MouseEvent as ReactMouseEvent, type PointerEvent as ReactPointerEvent } from 'react';
import { ArrowDownUp, Download, FileText, Folder, FolderPlus, RefreshCw, Trash2, Upload, UserRound } from 'lucide-react';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import { getActiveTabId, getCurrentSessionId, subscribe } from '../../lib/app-state';
import { Button } from '../../components/ui/button';
import { Checkbox } from '../../components/ui/checkbox';
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '../../components/ui/dialog';
import { Input } from '../../components/ui/input';
import { Progress } from '../../components/ui/progress';
import { joinPath, parentPath, resolveRemoteHome, useFileList, type FileEntry, type FileKind } from './useFileList';
import { FilePane, type FileContextMenuItem } from './FilePane';
import { remoteDeleteCommands } from './remote-delete';
import { alertDialog, confirmDialog, passwordDialog } from '../../lib/dialogs';
import sessionManager from '../../lib/session-manager';
import { getDeclinedUser, getFileIdentity, getRejectedSaved, recordSyncResult, subscribeFileIdentity, syncFileUser } from './file-user';
import { savedPasswordChoices, sudoErrorMessage } from './sudo-upload';
import { deletePrompt } from './delete-prompt';
import { useFileSelectionFallback } from './shortcuts';
import { useTransfer } from './useTransfer';
import type { TransferStatusStore } from './transfer-progress';
import { createDragPreviewStore, dragItems, dragLabel, dropPointToCss, dropTargetFrom, planDrop, releasedMove, sameDropTarget, type DragPreviewStore, type DropTarget } from './file-drag';

const subscribeActiveTabId = (callback: () => void) => subscribe('activeTabId', callback);
const subscribeCurrentSessionId = (callback: () => void) => subscribe('currentSessionId', callback);

type AppWindow = Window & { api?: any; fileManager?: any; connectionManager?: any };
type PaneName = 'local' | 'remote';
type RowDrag = { pane: PaneName; items: FileEntry[]; pointerId: number; startX: number; startY: number; lastX: number; lastY: number; active: boolean };
const DRAG_THRESHOLD = 5;
const IS_MAC = typeof navigator !== 'undefined' && /Mac/.test(navigator.platform);
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

// 传输进度条自己订阅进度：传输期间约每 100ms 一次的进度只重渲染这一条，不连带两侧文件列表
function TransferStatusBar({ store }: { store: TransferStatusStore }) {
  const status = useSyncExternalStore(store.subscribe, store.get);
  if (!status.active) return null;
  return (
    <div className="flex h-11 shrink-0 items-center gap-3 rounded-xl border border-border bg-card px-3.5 text-xs">
      <span className="flex h-[26px] w-[26px] shrink-0 items-center justify-center rounded-lg bg-accent text-primary" aria-hidden="true"><ArrowDownUp className="h-3.5 w-3.5" /></span>
      <span className="min-w-0 flex-1 truncate">{status.info}</span>
      <Progress value={status.progress} className="h-1.5 w-48 shrink-0" />
    </div>
  );
}

// 拖拽中跟着指针的提示：拖了什么、松手会传到哪。自己订阅指针位置，拖拽时只有它按指针频率重渲染
function DragPreviewOverlay({ store }: { store: DragPreviewStore }) {
  const preview = useSyncExternalStore(store.subscribe, store.get);
  if (!preview) return null;
  return (
    <div className="pointer-events-none fixed z-50 flex max-w-80 items-center gap-1.5 rounded-lg border border-border bg-popover px-2.5 py-1.5 text-xs text-popover-foreground shadow-lg" style={{ left: preview.x + 14, top: preview.y + 14 }}>
      {preview.items.length === 1 && !preview.items[0].isDirectory ? <FileText className="h-3.5 w-3.5 shrink-0 text-muted-foreground" /> : <Folder className="h-3.5 w-3.5 shrink-0 text-primary" />}
      <span className="truncate font-medium">{dragLabel(preview.items)}</span>
      {preview.destination && (
        <span className="shrink-0 text-muted-foreground">
          {preview.pane === 'local' ? '上传到' : '下载到'} {basename(preview.destination) || '/'}
        </span>
      )}
    </div>
  );
}

// 放置目标（高亮哪一侧、哪个文件夹行）只在真正换了时才进 state：拖拽时指针每动一下都 setState
// 会让整个文件管理和两侧列表按指针频率重渲染
function useDropTarget() {
  const currentRef = useRef<DropTarget | null>(null);
  const [target, setTarget] = useState<DropTarget | null>(null);
  const update = useCallback((next: DropTarget | null) => {
    if (sameDropTarget(currentRef.current, next)) return;
    currentRef.current = next;
    setTarget(next);
  }, []);
  return [target, update] as const;
}

// 没有 props、数据全来自外部 store（useSyncExternalStore）和自身状态：memo 后 App 重渲染（连接过程中多次）不再连带它
export const FileManager = memo(function FileManager() {
  const activeTabId = useSyncExternalStore(subscribeActiveTabId, getActiveTabId, () => 'terminal');
  const sessionId = useSyncExternalStore(subscribeCurrentSessionId, getCurrentSessionId, () => null);
  const enabled = activeTabId === 'file-manager';
  const local = useFileList({ kind: 'local', sessionId: null, enabled });
  const remote = useFileList({ kind: 'remote', sessionId, enabled });
  const [selected, setSelected] = useState<Selected>({ local: new Set(), remote: new Set() });
  const [lastSelected, setLastSelected] = useState<{ pane: PaneName; index: number } | null>(null);
  // 最近操作过的一侧：焦点不在列表上时 Cmd+A 全选这一侧；都没碰过时已连接选远程，否则选本地。
  const activePane = useRef<PaneName | null>(null);
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
    if (suppressClickRef.current) return;
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

  // 全选当前目录的全部条目（含虚拟化未渲染的行），并清空另一侧，与点选规则一致。
  const selectAll = useCallback((pane: PaneName) => {
    const entries = pane === 'local' ? local.files : remote.files;
    setSelected(current => ({ ...current, [pane]: new Set(entries.map(item => item.path)), [pane === 'local' ? 'remote' : 'local']: new Set() }));
  }, [local.files, remote.files]);
  useFileSelectionFallback(enabled, {
    selectAll: () => selectAll(activePane.current || (sessionId ? 'remote' : 'local')),
    clearSelection: () => setSelected({ local: new Set(), remote: new Set() }),
  });

  const ensureContextSelection = useCallback((pane: PaneName, entry: FileEntry | null) => {
    setContextTargets(current => ({ ...current, [pane]: entry }));
    if (!entry) return;
    setSelected(current => current[pane].has(entry.path)
      ? current
      : { ...current, [pane]: new Set([entry.path]), [pane === 'local' ? 'remote' : 'local']: new Set() });
  }, []);

  // 文件管理跟随终端用户（file-user.ts）：切到文件管理 / 刷新时看终端此刻是谁，变了就切换身份并回到他的主目录。
  const identity = useSyncExternalStore(subscribeFileIdentity, () => getFileIdentity(sessionId), () => null);
  const syncingRef = useRef(false);
  const syncIdentity = useCallback(async (interactive: boolean) => {
    if (!sessionId || syncingRef.current) return;
    syncingRef.current = true;
    try {
      const api = (window as AppWindow).api;
      const connectionId = sessionManager.getSession(sessionId)?.connectionId;
      let loginUser = '';
      let terminalUser = '';
      const declinedUser = getDeclinedUser(sessionId);
      const result = await syncFileUser({
        interactive,
        declinedUser,
        rejectedSaved: getRejectedSaved(sessionId),
        identify: async () => {
          const who = await api.file.terminalUser(sessionId);
          if (!who?.success) return null;
          loginUser = who.loginUser;
          terminalUser = who.user;
          return who;
        },
        switchUser: (target, secret) => api.file.switchUser(sessionId, target, secret),
        askPassword: async (retry, exclude, context) => {
          const saved = connectionId ? await api.cred.list(connectionId).catch(() => null) : null;
          const choices = savedPasswordChoices(saved, context.needUser, exclude, context.loginUser);
          const answer = await passwordDialog({
            title: retry ? `${context.needUser} 的密码不正确` : `以 ${context.user} 身份管理文件`,
            message: context.via === 'su'
              ? `终端已切换到 ${context.user}，文件管理也以 ${context.user} 身份浏览、上传和下载，需要 ${context.user} 的密码。`
              : `终端用 sudo 切换到了 ${context.user}，文件管理也以 ${context.user} 身份浏览、上传和下载，需要 ${context.loginUser} 的 sudo 密码。`,
            placeholder: choices.length ? '或输入密码' : '输入密码',
            confirmLabel: '切换',
            choices,
          });
          return answer === null ? null : 'choice' in answer ? { credential: answer.choice } : answer;
        },
      });
      // 会话在等密码时已被移除：丢弃结果，不写记录、不刷新、不弹报错
      if (!recordSyncResult(sessionId, result, { loginUser, terminalUser, declinedUser })) return;
      if (result.status === 'switched') {
        remote.clearCache();
        void remote.load(await resolveRemoteHome(sessionId) || '/', true);
      } else if (result.status === 'failed') {
        void alertDialog({ title: `无法以 ${terminalUser} 身份管理文件`, message: sudoErrorMessage(result.error) });
      }
    } finally {
      syncingRef.current = false;
    }
  }, [remote, sessionId]);
  const syncIdentityRef = useRef(syncIdentity);
  syncIdentityRef.current = syncIdentity;
  useEffect(() => {
    if (enabled && sessionId) void syncIdentityRef.current(false);
  }, [enabled, sessionId]);

  const refreshLocal = useCallback(() => { void local.load(local.path, true); }, [local]);
  const refreshRemote = useCallback(() => {
    if (!sessionId) return;
    void remote.load(remote.path || '/', true);
    void syncIdentityRef.current(false);
  }, [remote, sessionId]);

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

  const uploadItems = useCallback(async (items: FileEntry[], directory = remote.path) => {
    if (!sessionId) return;
    for (const item of items) {
      const destination = joinPath(directory, basename(item.path));
      if (item.isDirectory) await transfer.uploadDirectory(item.path, destination);
      else await transfer.uploadFile(item.path, destination);
    }
  }, [remote.path, sessionId, transfer]);

  const downloadItems = useCallback(async (items: FileEntry[], directory = local.path) => {
    if (!sessionId || !directory) return;
    for (const item of items) {
      if (item.isDirectory) await transfer.downloadDirectory(item.path, joinPath(directory, basename(item.path)));
      else await transfer.downloadFile(item.path, joinPath(directory, basename(item.path)));
    }
  }, [local.path, sessionId, transfer]);

  // 拖拽：两侧互拖 = 上传 / 下载（指针事件），从 Finder 拖进远程 = 上传（Tauri 拖放事件，带本地路径）。见 file-drag.ts。
  const rowDragRef = useRef<RowDrag | null>(null);
  const rootRef = useRef<HTMLDivElement>(null);
  const suppressClickRef = useRef(false);
  // 指针位置只进浮层的 store；两侧列表只看放置目标（换了才重渲染）
  const [dragPreview] = useState(createDragPreviewStore);
  const [rowTarget, setRowTarget] = useDropTarget();
  const [finderTarget, setFinderTarget] = useDropTarget();
  const dragContext = useRef({ dirs: { local: local.path, remote: remote.path }, sessionId, uploadItems, downloadItems, transfer });
  dragContext.current = { dirs: { local: local.path, remote: remote.path }, sessionId, uploadItems, downloadItems, transfer };

  const beginRowDrag = useCallback((pane: PaneName, entry: FileEntry, event: ReactPointerEvent<HTMLTableRowElement>) => {
    if (event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || !sessionId) return;
    const items = dragItems(entry, selected[pane], pane === 'local' ? local.files : remote.files);
    if (!items.length) return;
    rowDragRef.current = { pane, items, pointerId: event.pointerId, startX: event.clientX, startY: event.clientY, lastX: event.clientX, lastY: event.clientY, active: false };
  }, [local.files, remote.files, selected, sessionId]);

  useEffect(() => {
    const targetAt = (x: number, y: number) => dropTargetFrom(document.elementFromPoint(x, y));
    const cancel = () => {
      if (!rowDragRef.current) return;
      rowDragRef.current = null;
      dragPreview.set(null);
      setRowTarget(null);
    };
    const move = (event: PointerEvent) => {
      const drag = rowDragRef.current;
      if (!drag || drag.pointerId !== event.pointerId) return;
      if ((event.buttons & 1) === 0) {
        // 左键已松开却没收到 pointerup（实测 WKWebView 拖拽后常丢 pointerup / mouseup）：见 releasedMove
        if (releasedMove(drag, event.clientX, event.clientY, DRAG_THRESHOLD) === 'drop') finish(event);
        else cancel();
        return;
      }
      // 阈值之下算点击，轻碰一下不进入拖拽
      if (!drag.active && Math.hypot(event.clientX - drag.startX, event.clientY - drag.startY) < DRAG_THRESHOLD) return;
      if (!drag.active) {
        // 捕获指针（同会话标签、连接树的拖拽）：实测 pointerup 仍会丢，但捕获后松手时立刻补来那个
        // 松开状态的 pointermove（不捕获时要等下一次移动鼠标），releasedMove 据此当场完成拖放
        try { rootRef.current?.setPointerCapture(event.pointerId); } catch {}
      }
      drag.active = true;
      drag.lastX = event.clientX;
      drag.lastY = event.clientY;
      event.preventDefault();
      const target = targetAt(event.clientX, event.clientY);
      const action = planDrop(drag.pane, target, dragContext.current.dirs);
      dragPreview.set({ pane: drag.pane, items: drag.items, x: event.clientX, y: event.clientY, destination: action?.directory ?? null });
      setRowTarget(action ? target : null);
    };
    // 收尾挂 window：指针可能在面板外松开
    const finish = (event: PointerEvent) => {
      const drag = rowDragRef.current;
      if (!drag || drag.pointerId !== event.pointerId) return;
      rowDragRef.current = null;
      dragPreview.set(null);
      setRowTarget(null);
      try { rootRef.current?.releasePointerCapture(drag.pointerId); } catch {}
      if (!drag.active || event.type === 'pointercancel') return;
      // 拖完松手别再当成点了一下这一行
      suppressClickRef.current = true;
      setTimeout(() => { suppressClickRef.current = false; }, 0);
      const action = planDrop(drag.pane, targetAt(event.clientX, event.clientY), dragContext.current.dirs);
      if (action?.kind === 'upload') void dragContext.current.uploadItems(drag.items, action.directory);
      else if (action?.kind === 'download') void dragContext.current.downloadItems(drag.items, action.directory);
    };
    window.addEventListener('pointermove', move, true);
    window.addEventListener('pointerup', finish, true);
    window.addEventListener('pointercancel', finish, true);
    // 切到别的应用（窗口失焦）时拖拽作废
    window.addEventListener('blur', cancel);
    return () => {
      window.removeEventListener('pointermove', move, true);
      window.removeEventListener('pointerup', finish, true);
      window.removeEventListener('pointercancel', finish, true);
      window.removeEventListener('blur', cancel);
    };
  }, [dragPreview, setRowTarget]);

  useEffect(() => {
    if (!enabled) return;
    let stop: (() => void) | null = null;
    let disposed = false;
    const targetAt = (position: { x: number; y: number }) => {
      const point = dropPointToCss(position, IS_MAC, window.devicePixelRatio || 1);
      const target = dropTargetFrom(document.elementFromPoint(point.x, point.y));
      const action = dragContext.current.sessionId ? planDrop('finder', target, dragContext.current.dirs) : null;
      return { target: action ? target : null, action };
    };
    void getCurrentWebview().onDragDropEvent(async event => {
      const payload = event.payload;
      if (payload.type === 'leave') { setFinderTarget(null); return; }
      const { target, action } = targetAt(payload.position);
      if (payload.type !== 'drop') { setFinderTarget(target); return; }
      setFinderTarget(null);
      if (!action || !payload.paths.length) return;
      const kinds: boolean[] = await (window as AppWindow).api.file.localKinds(payload.paths).catch(() => payload.paths.map(() => false));
      const { transfer: current } = dragContext.current;
      for (const [index, localPath] of payload.paths.entries()) {
        const destination = joinPath(action.directory, basename(localPath));
        if (kinds[index]) await current.uploadDirectory(localPath, destination);
        else await current.uploadFile(localPath, destination);
      }
    }).then(unlisten => { if (disposed) unlisten(); else stop = unlisten; }).catch(() => {});
    return () => { disposed = true; stop?.(); setFinderTarget(null); };
  }, [enabled, setFinderTarget]);

  const deleteItems = useCallback(async (pane: PaneName, items: FileEntry[]) => {
    if (!items.length) return;
    if (!await confirmDialog(deletePrompt(items))) return;

    if (pane === 'remote') {
      for (const command of remoteDeleteCommands(items.map(item => item.path))) {
        // 以文件管理当前身份删除（终端切成 root 后就以 root 删）
        const result = await (window as AppWindow).api.file.shell(sessionId, command);
        if (!result?.success) void alertDialog({ title: '删除失败', message: result?.error || '未知错误' });
      }
      remote.invalidate(remote.path, sessionId);
      void remote.load(remote.path || '/', true);
      return;
    }
    for (const item of items) {
      const result = await (window as AppWindow).api.file[item.isDirectory ? 'deleteLocalDirectory' : 'deleteLocal'](item.path);
      if (!result?.success) {
        void alertDialog({ title: '删除失败', message: result?.error || '未知错误' });
        continue;
      }
      local.invalidate(local.path);
    }
    void local.load(local.path, true);
  }, [local, remote, sessionId]);

  const createRemoteDirectory = useCallback(async () => {
    if (!sessionId) return;
    const name = window.prompt('请输入文件夹名称');
    if (!name) return;
    if (name.includes('/') || name.includes('\\')) {
      void alertDialog({ title: '无法新建文件夹', message: '文件夹名称不能包含斜杠。' });
      return;
    }
    const fullPath = remote.path === '/' ? `/${name}` : `${remote.path}/${name}`;
    const result = await (window as AppWindow).api.file.createRemoteDirectory(sessionId, fullPath);
    if (result?.success) {
      remote.invalidate(remote.path, sessionId);
      void remote.load(remote.path || '/', true);
    } else {
      void alertDialog({ title: '新建文件夹失败', message: result?.error || '未知错误' });
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
      void alertDialog({ title: '修改权限失败', message: result?.error || '未知错误' });
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
    <div ref={rootRef} className="relative flex h-full min-h-0 flex-col gap-2.5" data-file-manager>
      <div className="flex min-h-0 flex-1 overflow-hidden" id="file-manager-split-pane">
        <div className="flex min-h-0 min-w-0 flex-1" data-file-pane="local" onMouseDownCapture={() => { activePane.current = 'local'; }}>
          <FilePane
            kind="local"
            path={local.path}
            files={local.files}
            loading={local.loading}
            emptyMessage={local.error}
            selectedPaths={selected.local}
            contextMenuItems={menuItems('local')}
            onPathChange={local.setPath}
            onPathSubmit={path => void local.load(path || local.path, true)}
            onListDirectory={local.listDirectory}
            onHome={async () => { const home = await (window as AppWindow).api.file.getHomeDir(); if (home) void local.load(home, true); }}
            onRefresh={refreshLocal}
            onBrowse={() => void local.browse()}
            onOpen={entry => openEntry('local', entry)}
            onRowSelect={(entry, index, event) => selectRow('local', entry, index, event)}
            onContextTarget={entry => ensureContextSelection('local', entry)}
            onSelectAll={() => selectAll('local')}
            onClearSelection={() => clearSelection('local')}
            onRowPointerDown={(entry, event) => beginRowDrag('local', entry, event)}
            dropActive={rowTarget?.pane === 'local'}
            dropDirectory={rowTarget?.directory}
          />
        </div>
        <div id="pane-divider" className="w-2.5 shrink-0 cursor-col-resize rounded-full transition-colors hover:bg-primary/15" title="拖动调整面板宽度" />
        <div className="flex min-h-0 min-w-0 flex-1" data-file-pane="remote" onMouseDownCapture={() => { activePane.current = 'remote'; }}>
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
            onPathSubmit={path => void remote.load(path || remote.path, true)}
            onListDirectory={remote.listDirectory}
            onHome={async () => {
              if (!sessionId) return;
              const home = await resolveRemoteHome(sessionId);
              if (home) void remote.load(home, true);
            }}
            onRefresh={refreshRemote}
            onOpen={entry => openEntry('remote', entry)}
            onRowSelect={(entry, index, event) => selectRow('remote', entry, index, event)}
            onContextTarget={entry => ensureContextSelection('remote', entry)}
            onPermission={openPermissions}
            onSelectAll={() => selectAll('remote')}
            onClearSelection={() => clearSelection('remote')}
            onRowPointerDown={(entry, event) => beginRowDrag('remote', entry, event)}
            dropActive={rowTarget?.pane === 'remote' || finderTarget?.pane === 'remote'}
            dropDirectory={(rowTarget ?? finderTarget)?.directory}
            badge={sessionId && identity && (identity.fileUser !== identity.loginUser ? (
              <span className="flex h-6 shrink-0 items-center gap-1 rounded-md bg-primary/10 px-2 text-xs font-medium text-primary" title={`文件管理以 ${identity.fileUser} 身份运行（跟随终端）`}>
                <UserRound className="h-3.5 w-3.5" />{identity.fileUser}
              </span>
            ) : identity.terminalUser && identity.terminalUser !== identity.fileUser ? (
              <button type="button" className="flex h-6 shrink-0 items-center gap-1 rounded-md border border-border px-2 text-xs text-muted-foreground hover:bg-muted" title={`终端当前是 ${identity.terminalUser}，点击让文件管理也切过去`} onClick={() => void syncIdentity(true)}>
                <UserRound className="h-3.5 w-3.5" />切到 {identity.terminalUser}
              </button>
            ) : null)}
          />
        </div>
      </div>

      <DragPreviewOverlay store={dragPreview} />

      <TransferStatusBar store={transfer.statusStore} />

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
});
