import { useCallback, useEffect, useRef, useState } from 'react';
import sessionManager from '../../lib/session-manager';

export type FileKind = 'local' | 'remote';

export type FileEntry = {
  name: string;
  path: string;
  isDirectory: boolean;
  size?: number;
  modifyTime?: number;
  permissions?: number;
  owner?: string;
  group?: string;
  isParent?: boolean;
};

type AppWindow = Window & { api?: any };

type Cache<T> = {
  get: (key: string) => T | null;
  set: (key: string, value: T) => void;
  delete: (key: string) => void;
  clear: () => void;
};

function createCache<T>(maxSize: number): Cache<T> {
  const values = new Map<string, T>();
  return {
    get(key) {
      const value = values.get(key);
      if (value === undefined) return null;
      values.delete(key);
      values.set(key, value);
      return value;
    },
    set(key, value) {
      values.delete(key);
      if (values.size >= maxSize) values.delete(values.keys().next().value!);
      values.set(key, value);
    },
    delete(key) {
      values.delete(key);
    },
    clear() {
      values.clear();
    },
  };
}

const remoteFileCache = createCache<FileEntry[]>(50);
const localFileCache = createCache<FileEntry[]>(50);
let lastLocalDirectory: string | null = null;

export function sortFiles(files: FileEntry[]) {
  return [...files].sort((a, b) => {
    if (a.isDirectory !== b.isDirectory) return a.isDirectory ? -1 : 1;
    return a.name.localeCompare(b.name);
  });
}

export function normalizeRemotePath(value: string) {
  const path = value.replace(/\/+/g, '/');
  return `/${path}`.replace(/\/+/g, '/') || '/';
}

export function joinPath(directory: string, name: string) {
  const separator = directory.includes('\\') ? '\\' : '/';
  return directory.endsWith(separator) ? `${directory}${name}` : `${directory}${separator}${name}`;
}

export function parentPath(path: string, kind: FileKind) {
  if (kind === 'remote') {
    const clean = normalizeRemotePath(path).replace(/\/+$/, '') || '/';
    if (clean === '/') return '/';
    const index = clean.lastIndexOf('/');
    return index <= 0 ? '/' : clean.slice(0, index);
  }

  const separator = path.includes('\\') ? '\\' : '/';
  const clean = path.replace(/[\\/]+$/, '');
  const index = Math.max(clean.lastIndexOf('/'), clean.lastIndexOf('\\'));
  if (index <= 0) return separator;
  if (/^[A-Za-z]:$/.test(clean.slice(0, index))) return `${clean.slice(0, index)}${separator}`;
  return clean.slice(0, index);
}

export function isRootPath(path: string, kind: FileKind) {
  return kind === 'remote' ? normalizeRemotePath(path) === '/' : path === '/' || /^[A-Za-z]:[\\/]?$/.test(path);
}

export function formatFileSize(bytes = 0) {
  if (bytes === 0) return '0 B';
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  const index = Math.floor(Math.log(bytes) / Math.log(1024));
  return `${(bytes / Math.pow(1024, index)).toFixed(2)} ${units[index] || 'TB'}`;
}

// 固定宽度的列里日期要可预测：省掉秒，用零填充的两位数，避免宽度随内容跳动。
const dateFormatter = new Intl.DateTimeFormat(undefined, {
  year: 'numeric', month: '2-digit', day: '2-digit',
  hour: '2-digit', minute: '2-digit', hour12: false,
});

export function formatDate(value?: number) {
  return value ? dateFormatter.format(new Date(value)) : '-';
}

export function formatPermissions(mode?: number) {
  return mode ? mode.toString(8).slice(-3) : '-';
}

export function useFileList({ kind, sessionId, enabled }: { kind: FileKind; sessionId: string | null; enabled: boolean }) {
  const initialPath = kind === 'remote'
    ? (sessionId ? sessionManager.getRemotePath(sessionId) : '')
    : (lastLocalDirectory || '');
  const [path, setPathState] = useState(initialPath);
  const pathRef = useRef(initialPath);
  const requestRef = useRef(0);
  const [files, setFiles] = useState<FileEntry[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [emptyMessage, setEmptyMessage] = useState<string | null>(kind === 'remote' && !sessionId ? '请先连接到服务器' : null);

  const setPath = useCallback((nextPath: string) => {
    pathRef.current = nextPath;
    setPathState(nextPath);
  }, []);

  const clearCache = useCallback(() => {
    if (kind === 'remote') remoteFileCache.clear();
    else localFileCache.clear();
  }, [kind]);

  const invalidate = useCallback((directory: string, cacheSessionId = sessionId) => {
    if (kind === 'remote') {
      if (cacheSessionId) remoteFileCache.delete(`${cacheSessionId}:${normalizeRemotePath(directory)}`);
    } else {
      localFileCache.delete(directory);
    }
  }, [kind, sessionId]);

  const load = useCallback(async (requestedPath?: string, force = false) => {
    const appWindow = window as AppWindow;
    const requestId = ++requestRef.current;
    let nextPath = requestedPath || pathRef.current;

    try {
      if (kind === 'remote') {
        if (!sessionId) {
          setFiles([]);
          setPath('');
          setEmptyMessage('请先连接到服务器');
          return;
        }
        nextPath = normalizeRemotePath(nextPath || sessionManager.getRemotePath(sessionId) || '/');
        setPath(nextPath);
        sessionManager.updateRemotePath(sessionId, nextPath);
        setEmptyMessage(null);
      } else {
        if (!nextPath) {
          const selection = await appWindow.api?.dialog?.selectDirectory();
          if (selection?.canceled) {
            if (!lastLocalDirectory) return;
            nextPath = lastLocalDirectory;
          } else {
            nextPath = selection?.filePaths?.[0];
          }
        }
        if (!nextPath) return;
        lastLocalDirectory = nextPath;
        setPath(nextPath);
      }

      const path = nextPath;
      const cacheKey = kind === 'remote' ? `${sessionId}:${path}` : path;
      const cache = kind === 'remote' ? remoteFileCache : localFileCache;
      const cached = !force ? cache.get(cacheKey) : null;
      if (cached) setFiles(sortFiles(cached));

      setLoading(!cached);
      setError(null);
      const result = kind === 'remote'
        ? await appWindow.api?.file?.list(String(sessionId), nextPath)
        : await appWindow.api?.file?.listLocal(nextPath);

      if (requestId !== requestRef.current) return;
      if (result?.success) {
        const nextFiles = result.files || [];
        cache.set(cacheKey, nextFiles);
        if (kind === 'remote' && sessionManager.getRemotePath(String(sessionId)) !== path) return;
        setFiles(sortFiles(nextFiles));
      } else {
        throw new Error(result?.error || '获取文件列表失败');
      }
    } catch (cause: any) {
      if (requestId !== requestRef.current) return;
      const message = cause?.message || String(cause);
      setError(message);
      if (kind === 'remote' && nextPath !== '/') {
        const parent = parentPath(nextPath, kind);
        if (parent !== nextPath) {
          await load(parent, force);
          return;
        }
      }
    } finally {
      if (requestId === requestRef.current) setLoading(false);
    }
  }, [kind, sessionId, setPath]);

  const browse = useCallback(async () => {
    if (kind !== 'local') return;
    const result = await (window as AppWindow).api?.dialog?.selectDirectory();
    if (!result?.canceled && result.filePaths?.[0]) await load(result.filePaths[0], true);
  }, [kind, load]);

  useEffect(() => {
    requestRef.current += 1;
    const nextPath = kind === 'remote'
      ? (sessionId ? sessionManager.getRemotePath(sessionId) || '/' : '')
      : (lastLocalDirectory || '');
    pathRef.current = nextPath;
    setPathState(nextPath);
    setFiles([]);
    setError(null);
    setEmptyMessage(current => kind === 'remote' && !sessionId ? (current || '请先连接到服务器') : null);
    if (enabled) {
      if (kind === 'local' && !nextPath) {
        const getHomeDir = (window as AppWindow).api?.file?.getHomeDir;
        if (getHomeDir) void Promise.resolve(getHomeDir()).then((home: string) => load(home, false));
      } else {
        void load(nextPath || undefined);
      }
    }
  }, [enabled, kind, sessionId, load]);

  return {
    path,
    setPath,
    files,
    loading,
    error,
    emptyMessage,
    setEmptyMessage,
    load,
    browse,
    clearCache,
    invalidate,
  };
}

