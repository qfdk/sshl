import { useCallback, useEffect, useRef, useState } from 'react';
import {
  getActiveTabId,
  getCurrentSessionId,
  setConnectionDialogOpen,
  setCurrentSessionId,
  setEditingConnection,
} from '../../lib/app-state';
import { getUngroupedConnections, groupConnections, matchesConnection, moveConnection } from '../../lib/connection-groups';
import sessionManager from '../../lib/session-manager';
import type { TerminalManager } from '../terminal/useTerminal';
type Connection = { id: string; name?: string; host?: string; username?: string; group?: string; [key: string]: unknown };

const COLLAPSED_GROUPS_KEY = 'sshl.collapsedConnectionGroups';

type ConnectionDetails = {
  name: string;
  host: string;
  port: string;
  username: string;
  group: string;
  authType: 'password' | 'privateKey';
  password: string;
  privateKey: string;
  passphrase: string;
  savePassword: boolean;
};

type Attempt = { id: string; cancelled: boolean; resolved: boolean; sessionId: string | null };
type ApiWindow = Window & { terminalManager?: TerminalManager; sessionManager?: typeof sessionManager; connectionManager?: ConnectionManager; fileManager?: any; uiManager?: any };

export type ConnectionManager = {
  connections: Connection[];
  groupOrder: string[];
  revision: number;
  loadingText: string | null;
  isConnecting: boolean;
  cancel: () => Promise<void>;
  loadConnections: () => Promise<void>;
  connectToSaved: (id: string) => Promise<void>;
  switchToSession: (connectionId: string) => Promise<boolean>;
  submitConnection: (details: ConnectionDetails) => Promise<void>;
  saveEditedConnection: (id: string, details: ConnectionDetails) => Promise<void>;
  disconnect: (sessionId: string) => Promise<void>;
  deleteConnection: (id: string) => Promise<void>;
  move: (connectionId: string, targetGroup: string, beforeId?: string | null) => void;
  toggleGroup: (groupName: string) => void;
  isGroupCollapsed: (groupName: string) => boolean;
};

function appWindow() { return window as ApiWindow; }
function errorText(error: unknown) { return (error as { message?: string })?.message || String(error || '未知错误'); }

export function useConnections(): ConnectionManager {
  const [connections, setConnections] = useState<Connection[]>([]);
  const [groupOrder, setGroupOrder] = useState<string[]>([]);
  const [revision, setRevision] = useState(0);
  const [loadingText, setLoadingText] = useState<string | null>(null);
  const [isConnecting, setIsConnecting] = useState(false);
  const [collapsedGroups, setCollapsedGroups] = useState<Set<string>>(() => {
    try {
      const saved = JSON.parse(localStorage.getItem(COLLAPSED_GROUPS_KEY) || '[]');
      return new Set(Array.isArray(saved) ? saved : []);
    } catch { return new Set(); }
  });
  const activeAttempt = useRef<Attempt | null>(null);
  const connectionsRef = useRef(connections);
  const groupOrderRef = useRef(groupOrder);
  connectionsRef.current = connections;
  groupOrderRef.current = groupOrder;

  const loadConnections = useCallback(async () => {
    try {
      const api = appWindow().api;
      const [nextConnections, nextGroupOrder] = await Promise.all([
        api.config.getConnections(),
        api.config.getConnectionGroups(),
      ]);
      setConnections(Array.isArray(nextConnections) ? nextConnections : []);
      setGroupOrder(Array.isArray(nextGroupOrder) ? nextGroupOrder : []);
      setRevision(value => value + 1);
    } catch (error) {
      console.error('加载连接失败:', error);
    }
  }, []);

  const beginConnectionAttempt = useCallback(() => {
    const id = window.crypto?.randomUUID ? window.crypto.randomUUID() : `${Date.now()}-${Math.random().toString(16).slice(2)}`;
    const attempt: Attempt = { id, cancelled: false, resolved: false, sessionId: null };
    activeAttempt.current = attempt;
    setIsConnecting(true);
    setLoadingText('正在连接服务器...');
    return attempt;
  }, []);

  const cancelConnectionAttempt = useCallback(async () => {
    const attempt = activeAttempt.current;
    if (!attempt || attempt.cancelled || attempt.resolved) return;
    attempt.cancelled = true;
    if (activeAttempt.current === attempt) {
      activeAttempt.current = null;
      setIsConnecting(false);
    }
    setLoadingText(null);
    if (attempt.sessionId) {
      await appWindow().api.ssh.disconnect(attempt.sessionId);
      return;
    }
    if (appWindow().api.ssh.cancelConnect) await appWindow().api.ssh.cancelConnect(attempt.id);
  }, []);

  const waitForConnection = useCallback(async (details: unknown, attempt: Attempt) => {
    const result = await appWindow().api.ssh.connect(details, attempt.id);
    if (result?.success) attempt.sessionId = result.sessionId;
    attempt.resolved = true;
    if (attempt.cancelled) {
      if (attempt.sessionId) await appWindow().api.ssh.disconnect(attempt.sessionId);
      return null;
    }
    if (result?.success) setLoadingText('正在准备终端...');
    return result;
  }, []);

  const finishConnectionAttempt = useCallback((attempt: Attempt) => {
    if (activeAttempt.current !== attempt) return;
    activeAttempt.current = null;
    setIsConnecting(false);
    setLoadingText(null);
  }, []);

  const updateConnectionStatus = useCallback((connected: boolean, name = '', host = '') => {
    appWindow().uiManager?.updateConnectionStatus?.(connected, name);
    appWindow().uiManager?.updateServerInfo?.(connected, { name, host });
  }, []);

  const discardStaleSession = useCallback((sessionInfo: { sessionId: string; session?: { connectionId?: string } } | null) => {
    if (!sessionInfo) return;
    const { sessionId, session } = sessionInfo;
    sessionManager.removeSession(sessionId);
    appWindow().terminalManager?.disposeTerminalInstance(sessionId);
    if (getCurrentSessionId() === sessionId) {
      setCurrentSessionId(null);
      updateConnectionStatus(false);
      appWindow().fileManager?.clearCache?.();
      appWindow().fileManager?.showRemoteEmptyState?.('连接已失效，正在重新连接...', session?.connectionId || null);
    }
    setRevision(value => value + 1);
  }, [updateConnectionStatus]);

  const switchToSession = useCallback(async (connectionId: string) => {
    let sessionInfo = sessionManager.getSessionByConnectionId(connectionId);
    if (!sessionInfo) return false;
    if (appWindow().api.ssh.validateSession) {
      const alive = await appWindow().api.ssh.validateSession(sessionInfo.sessionId);
      if (!alive?.success || !alive.alive) {
        discardStaleSession(sessionInfo);
        return false;
      }
    }

    const activationResult = await appWindow().api.ssh.activateSession(sessionInfo.sessionId);
    if (!activationResult?.success) {
      discardStaleSession(sessionInfo);
      return false;
    }
    if (getCurrentSessionId() === sessionInfo.sessionId) return true;

    try {
      appWindow().fileManager?.clearCache?.();
      sessionInfo = sessionManager.getSessionByConnectionId(connectionId);
      if (!sessionInfo) return false;
      const session = sessionInfo.session;
      if (!session) {
        sessionManager.removeSession(sessionInfo.sessionId);
        appWindow().terminalManager?.disposeTerminalInstance(sessionInfo.sessionId);
        const currentConnections = await appWindow().api.config.getConnections();
        const connection = currentConnections.find((item: Connection) => item.id === connectionId);
        if (!connection) return false;
        try {
          const result = await appWindow().api.ssh.connect(connection);
          if (result.success) {
            setCurrentSessionId(result.sessionId);
            const terminalInfo = await appWindow().terminalManager!.initTerminal(result.sessionId, null, false);
            if (!terminalInfo) return false;
            sessionManager.addSession(result.sessionId, connectionId, { term: terminalInfo.term, buffer: '', name: connection.name });

            // 原子激活：缓冲快照由后端作为首个 ssh:data 事件发出，
            // 与后续数据同通道保序（invoke 返回快照会与事件乱序）
            try {
              await appWindow().api.ssh.activateSession(result.sessionId);
            } catch (err) {
              console.warn(`[switchToSession reconnect] activate 失败:`, err);
            }
            updateConnectionStatus(true, connection.name, connection.host);
            setRevision(value => value + 1);
            return true;
          }
          return false;
        } catch (error) {
          console.error('[switchToSession] 重新连接出错:', error);
          return false;
        }
      }

      if (getCurrentSessionId() && getCurrentSessionId() !== sessionInfo.sessionId) sessionManager.setSessionActive(getCurrentSessionId()!, false);
      setCurrentSessionId(sessionInfo.sessionId);
      sessionManager.setSessionActive(sessionInfo.sessionId, true);

      const terminalResult = await appWindow().terminalManager!.initTerminal(sessionInfo.sessionId, sessionInfo.session, false);
      if (!terminalResult) throw new Error('终端初始化失败');

      // 仅新建终端时拉缓冲区回放；复用则跳过，避免内容重复
      if (terminalResult.isNew) {
        try {
          const bufferResult = await appWindow().api.ssh.getSessionBuffer(sessionInfo.sessionId);
          if (bufferResult?.success && bufferResult.buffer) terminalResult.term.write(bufferResult.buffer);
        } catch (err) {
          console.warn(`[switchToSession] 加载缓冲区数据失败:`, err);
        }
      }
      const currentConnections = await appWindow().api.config.getConnections();
      const connection = currentConnections.find((item: Connection) => item.id === connectionId);
      if (connection) updateConnectionStatus(true, connection.name, connection.host);
      setRevision(value => value + 1);
      return true;
    } catch (error) {
      console.error('切换会话失败:', error);
      return false;
    }
  }, [discardStaleSession, updateConnectionStatus]);

  const connectToSaved = useCallback(async (id: string) => {
    if (activeAttempt.current || isConnecting) return;
    let attempt: Attempt | null = null;
    try {
      const currentConnections = await appWindow().api.config.getConnections();
      const connection = currentConnections.find((item: Connection) => item.id === id);
      if (!connection) return;
      const sessionInfo = sessionManager.getSessionByConnectionId(connection.id);
      if (sessionInfo) {
        if (sessionInfo.session && !sessionInfo.session.active) sessionManager.updateSession(sessionInfo.sessionId, { active: true });
        if (await switchToSession(connection.id)) return;
      }

      attempt = beginConnectionAttempt();
      const result = await waitForConnection(connection, attempt);
      if (!result) return;
      if (result.success) {
        setCurrentSessionId(result.sessionId);
        const terminalInfo = await appWindow().terminalManager!.initTerminal(result.sessionId, null, false);
        if (terminalInfo) sessionManager.addSession(result.sessionId, connection.id, { term: terminalInfo.term, buffer: '', name: connection.name });

        // 原子激活：缓冲快照由后端作为首个 ssh:data 事件发出，
        // 与后续数据同通道保序（invoke 返回快照会与事件乱序）
        try {
          await appWindow().api.ssh.activateSession(result.sessionId);
        } catch (err) {
          console.warn(`[连接] 激活会话失败:`, err);
        }
        updateConnectionStatus(true, connection.name, connection.host);
        await loadConnections();
      } else {
        alert(`连接失败: ${result?.error || 'unknown error'}`);
      }
    } catch (error) {
      if (attempt?.cancelled) return;
      console.error('连接错误:', error);
      alert(`连接错误: ${errorText(error)}`);
    } finally {
      if (attempt) finishConnectionAttempt(attempt);
    }
  }, [beginConnectionAttempt, finishConnectionAttempt, isConnecting, loadConnections, switchToSession, updateConnectionStatus, waitForConnection]);

  const submitConnection = useCallback(async (details: ConnectionDetails) => {
    if (activeAttempt.current || isConnecting) return;
    let attempt: Attempt | null = null;
    try {
      const authType = details.authType;
      const savePassword = Boolean(details.savePassword);
      const connectionDetails: Record<string, unknown> = { name: details.name, host: details.host, port: parseInt(details.port), username: details.username, group: (details.group || '').trim(), authType };
      if (authType === 'password') connectionDetails.password = details.password;
      else {
        connectionDetails.privateKey = details.privateKey;
        if (details.passphrase) connectionDetails.passphrase = details.passphrase;
      }
      attempt = beginConnectionAttempt();
      const result = await waitForConnection(connectionDetails, attempt);
      if (!result) return;
      if (result.success) {
        const generatedId = Date.now().toString();
        setCurrentSessionId(result.sessionId);
        const savedConnectionDetails = { ...connectionDetails };
        if (!savePassword) {
          if (authType === 'password') savedConnectionDetails.password = '';
          else if (savedConnectionDetails.passphrase) savedConnectionDetails.passphrase = '';
        }
        await appWindow().api.config.saveConnection({ ...savedConnectionDetails, id: generatedId, sessionId: result.sessionId });
        updateConnectionStatus(true, details.name, details.host);
        setConnectionDialogOpen(false);
        setEditingConnection(null);
        const terminalInfo = await appWindow().terminalManager!.initTerminal(result.sessionId, null, false);
        if (terminalInfo) sessionManager.addSession(result.sessionId, generatedId, { term: terminalInfo.term, buffer: '', name: details.name });

        // 原子激活：缓冲快照由后端作为首个 ssh:data 事件发出，
        // 与后续数据同通道保序（invoke 返回快照会与事件乱序）
        try {
          await appWindow().api.ssh.activateSession(result.sessionId);
        } catch (err) {
          console.warn(`[连接] 激活会话失败:`, err);
        }
        await loadConnections();
      } else alert(`连接失败: ${result.error}`);
    } catch (error) {
      if (attempt?.cancelled) return;
      console.error('连接错误:', error);
      alert(`连接错误: ${errorText(error)}`);
    } finally {
      if (attempt) finishConnectionAttempt(attempt);
    }
  }, [beginConnectionAttempt, finishConnectionAttempt, isConnecting, loadConnections, updateConnectionStatus, waitForConnection]);

  const saveEditedConnection = useCallback(async (id: string, details: ConnectionDetails) => {
    try {
      const authType = details.authType;
      const savePassword = Boolean(details.savePassword);
      const connectionDetails: Record<string, unknown> = { id, name: details.name, host: details.host, port: parseInt(details.port), username: details.username, group: (details.group || '').trim(), authType };
      if (authType === 'password') {
        if (savePassword) connectionDetails.password = details.password;
      } else {
        connectionDetails.privateKey = details.privateKey;
        if (savePassword && details.passphrase) connectionDetails.passphrase = details.passphrase;
      }
      const result = await appWindow().api.config.saveConnection(connectionDetails);
      if (result) {
        setConnectionDialogOpen(false);
        setEditingConnection(null);
        await loadConnections();
      } else alert('保存连接失败');
    } catch (error) {
      console.error('编辑连接失败:', error);
      alert(`编辑连接失败: ${errorText(error)}`);
    }
  }, [loadConnections]);

  const disconnect = useCallback(async (sessionId: string) => {
    if (!sessionId) return;
    const isActive = getCurrentSessionId() === sessionId;
    const connectionId = sessionManager.getSession(sessionId)?.connectionId || null;
    const fileManager = appWindow().fileManager;
    const transferring = fileManager && fileManager.activeTransfers > 0;
    const onFileTab = getActiveTabId() === 'file-manager';
    if (transferring || onFileTab) {
      const message = transferring ? '当前有正在进行的文件传输，断开连接会中断传输。\n确定要断开吗？' : '当前正在使用文件管理，断开连接会关闭文件传输。\n确定要断开吗？';
      if (!await appWindow().api.dialog.confirm(message, '断开连接')) return;
    }
    try { await appWindow().api.ssh.disconnect(sessionId); } catch (error) { console.error('断开连接失败:', error); }
    sessionManager.removeSession(sessionId);
    appWindow().terminalManager?.disposeTerminalInstance(sessionId);
    if (isActive) {
      setCurrentSessionId(null);
      updateConnectionStatus(false);
      fileManager?.clearFileManagerCache?.();
      fileManager?.renderRemoteEmptyState?.('连接已断开，请重新连接到服务器', connectionId);
    }
    setRevision(value => value + 1);
    await loadConnections();
  }, [loadConnections, updateConnectionStatus]);

  const deleteConnection = useCallback(async (id: string) => {
    if (!await appWindow().api.dialog.confirm('确定要删除这个连接吗?', '删除连接')) return;
    const result = await appWindow().api.config.deleteConnection(id);
    if (result) await loadConnections();
  }, [loadConnections]);

  const move = useCallback((connectionId: string, targetGroup: string, beforeId: string | null = null) => {
    const next = moveConnection(connectionsRef.current, groupOrderRef.current, connectionId, targetGroup, beforeId);
    setConnections(next);
    void appWindow().api.config.applyConnectionLayout(next.map(connection => ({ id: connection.id, group: connection.group?.trim() || '' })));
  }, []);

  const toggleGroup = useCallback((groupName: string) => {
    if (!groupName) return;
    setCollapsedGroups(previous => {
      const next = new Set(previous);
      if (next.has(groupName)) next.delete(groupName); else next.add(groupName);
      localStorage.setItem(COLLAPSED_GROUPS_KEY, JSON.stringify([...next]));
      return next;
    });
  }, []);

  const setupConnectionCancellation = useCallback(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      const attempt = activeAttempt.current;
      if (event.key !== 'Escape' || !attempt || attempt.cancelled || attempt.resolved) return;
      event.preventDefault();
      event.stopImmediatePropagation();
      void cancelConnectionAttempt();
    };
    document.addEventListener('keydown', onKeyDown, true);
    return () => document.removeEventListener('keydown', onKeyDown, true);
  }, [cancelConnectionAttempt]);

  useEffect(() => setupConnectionCancellation(), [setupConnectionCancellation]);

  useEffect(() => {
    const stopData = appWindow().api.ssh.onData((_event: unknown, data: { data: string; sessionId: string }) => {
      const terminal = appWindow().terminalManager?.getTerminalForSession(data.sessionId);
      if (terminal) {
        try { terminal.write(data.data); } catch (error) { console.error('[ssh:data] 写入终端失败:', error); }
      }
    });
    const stopClosed = appWindow().api.ssh.onClosed(async (_event: unknown, data: { sessionId: string }) => {
      const connectionId = sessionManager.getSession(data.sessionId)?.connectionId || null;
      sessionManager.setSessionActive(data.sessionId, false);
      sessionManager.removeSession(data.sessionId);
      appWindow().terminalManager?.disposeTerminalInstance(data.sessionId);
      if (data.sessionId === getCurrentSessionId()) {
        setCurrentSessionId(null);
        updateConnectionStatus(false);
        appWindow().fileManager?.clearCache?.();
        appWindow().fileManager?.showRemoteEmptyState?.('连接已断开，请重新连接到服务器', connectionId);
      }
      setRevision(value => value + 1);
      await loadConnections();
    });
    void loadConnections();
    const stopUpdated = appWindow().api.config.onConnectionsUpdated(() => { void loadConnections(); });
    return () => { stopData?.(); stopClosed?.(); stopUpdated?.(); };
  }, [loadConnections, updateConnectionStatus]);

  const manager: ConnectionManager = {
    connections,
    groupOrder,
    revision,
    loadingText,
    isConnecting,
    cancel: cancelConnectionAttempt,
    loadConnections,
    connectToSaved,
    switchToSession,
    submitConnection,
    saveEditedConnection,
    disconnect,
    deleteConnection,
    move,
    toggleGroup,
    isGroupCollapsed: groupName => collapsedGroups.has(groupName),
  };

  useEffect(() => {
    const target = appWindow();
    target.connectionManager = manager;
    target.sessionManager = sessionManager;
    return () => { if (target.connectionManager === manager) delete target.connectionManager; };
  });

  return manager;
}
