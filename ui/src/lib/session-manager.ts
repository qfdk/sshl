export type Session = {
  connectionId: string;
  buffer: string;
  active: boolean;
  currentRemotePath: string;
  [key: string]: unknown;
};

class SessionManager {
  sessions = new Map<string, Session>();
  private removedListeners = new Set<(sessionId: string) => void>();

  addSession(sessionId: string, connectionId: string, data: Partial<Session> = {}) {
    this.sessions.set(sessionId, {
      ...data,
      connectionId,
      active: true,
      buffer: data.buffer || '',
      lastActive: Date.now(),
      // 空 = 还没打开过文件管理；第一次打开时从远程 home 开始（useFileList）
      currentRemotePath: '',
    });
  }

  getSession(sessionId: string) {
    return this.sessions.get(sessionId);
  }

  getSessionByConnectionId(connectionId: string) {
    for (const [sessionId, session] of this.sessions.entries()) {
      if (session.connectionId === connectionId) return { sessionId, session };
    }
    return null;
  }

  updateSession(sessionId: string, data: Partial<Session>) {
    if (this.sessions.has(sessionId)) {
      const session = this.sessions.get(sessionId)!;
      this.sessions.set(sessionId, { ...session, ...data });
    }
  }

  removeSession(sessionId: string) {
    this.sessions.delete(sessionId);
    this.removedListeners.forEach(listener => listener(sessionId));
  }

  /** 会话被移除（主动断开、远端关闭、失效丢弃都走 removeSession）时通知，用来清掉按 sessionId 记的状态。 */
  onSessionRemoved(listener: (sessionId: string) => void) {
    this.removedListeners.add(listener);
    return () => { this.removedListeners.delete(listener); };
  }

  getAllSessions() {
    return Array.from(this.sessions.entries()).map(([id, session]) => ({ id, ...session }));
  }

  setSessionActive(sessionId: string, active: boolean) {
    if (this.sessions.has(sessionId)) {
      const session = this.sessions.get(sessionId)!;
      session.active = active;
      if (active) session.lastActive = Date.now();
      this.sessions.set(sessionId, session);
    } else {
      console.warn(`[sessionManager] 尝试设置不存在的会话 ${sessionId} 的活跃状态`);
    }
  }

  getRemotePath(sessionId: string) {
    return this.sessions.get(sessionId)?.currentRemotePath || '';
  }

  updateRemotePath(sessionId: string, path: string) {
    if (this.sessions.has(sessionId)) {
      const session = this.sessions.get(sessionId)!;
      session.currentRemotePath = path;
      this.sessions.set(sessionId, session);
    }
  }
}

const sessionManager = new SessionManager();
export default sessionManager;
