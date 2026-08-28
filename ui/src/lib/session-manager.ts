export type Session = {
  connectionId: string;
  buffer: string;
  active: boolean;
  currentRemotePath: string;
  [key: string]: unknown;
};

class SessionManager {
  sessions = new Map<string, Session>();

  addSession(sessionId: string, connectionId: string, data: Partial<Session> = {}) {
    this.sessions.set(sessionId, {
      ...data,
      connectionId,
      active: true,
      buffer: data.buffer || '',
      lastActive: Date.now(),
      currentRemotePath: '/',
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
  }

  updateSessionId(oldSessionId: string, newSessionId: string) {
    if (this.sessions.has(oldSessionId)) {
      this.sessions.set(newSessionId, this.sessions.get(oldSessionId)!);
      this.sessions.delete(oldSessionId);
    }
  }

  hasActiveSession(sessionId: string) {
    return this.sessions.has(sessionId) && this.sessions.get(sessionId)!.active;
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

  addToBuffer(sessionId: string, data: string) {
    if (this.sessions.has(sessionId)) {
      const session = this.sessions.get(sessionId)!;
      if (session.active) {
        const maxBufferSize = 100000;
        session.buffer = (session.buffer || '') + data;
        if (session.buffer.length > maxBufferSize) session.buffer = session.buffer.slice(-maxBufferSize);
        this.sessions.set(sessionId, session);
      }
    } else {
      console.warn(`[sessionManager] 尝试向不存在的会话 ${sessionId} 添加数据`);
    }
  }

  clearBuffer(sessionId: string) {
    if (this.sessions.has(sessionId)) {
      const session = this.sessions.get(sessionId)!;
      session.buffer = '';
      this.sessions.set(sessionId, session);
    } else {
      console.warn(`[sessionManager] 尝试清除不存在的会话 ${sessionId} 的缓冲区`);
    }
  }

  getRemotePath(sessionId: string) {
    return this.sessions.get(sessionId)?.currentRemotePath || '/';
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
