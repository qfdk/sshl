// session-manager.js
// 会话管理器，支持多个活跃会话的维护和切换

class SessionManager {
    constructor() {
        // 存储所有活动会话
        this.sessions = new Map();
    }

    // 添加会话
    addSession(sessionId, connectionId, data) {
        this.sessions.set(sessionId, {
            ...data,
            connectionId: connectionId,
            active: true,
            buffer: data.buffer || '',
            lastActive: Date.now(),
            currentRemotePath: '/' // 初始化远程工作目录为根目录
        });
    }

    // 获取会话
    getSession(sessionId) {
        return this.sessions.get(sessionId);
    }

    // 根据连接ID获取会话
    getSessionByConnectionId(connectionId) {
        for (const [sessionId, session] of this.sessions.entries()) {
            if (session.connectionId === connectionId) {
                return {sessionId, session};
            }
        }
        return null;
    }

    // 更新会话
    updateSession(sessionId, data) {
        if (this.sessions.has(sessionId)) {
            const session = this.sessions.get(sessionId);
            this.sessions.set(sessionId, {...session, ...data});
        }
    }

    // 移除会话
    removeSession(sessionId) {
        this.sessions.delete(sessionId);
    }

    // 更新会话ID（用于重新连接后的会话ID更新）
    updateSessionId(oldSessionId, newSessionId) {
        if (this.sessions.has(oldSessionId)) {
            const sessionData = this.sessions.get(oldSessionId);
            this.sessions.set(newSessionId, sessionData);
            this.sessions.delete(oldSessionId);
        }
    }

    // 会话是否存在且活跃
    hasActiveSession(sessionId) {
        return this.sessions.has(sessionId) && this.sessions.get(sessionId).active;
    }

    // 获取所有会话信息
    getAllSessions() {
        return Array.from(this.sessions.entries()).map(([id, session]) => ({
            id,
            ...session
        }));
    }

    // 设置会话为活跃或非活跃
    setSessionActive(sessionId, active) {
        if (this.sessions.has(sessionId)) {
            const session = this.sessions.get(sessionId);
            session.active = active;
            if (active) {
                session.lastActive = Date.now();
            }
            this.sessions.set(sessionId, session);
        } else {
            console.warn(`[sessionManager] 尝试设置不存在的会话 ${sessionId} 的活跃状态`);
        }
    }

    // 记录终端数据到缓冲区
    addToBuffer(sessionId, data) {
        if (this.sessions.has(sessionId)) {
            const session = this.sessions.get(sessionId);

            // 只有活跃会话才添加到缓冲区
            if (session.active) {
                // 防止缓冲区过大
                const maxBufferSize = 100000;
                session.buffer = (session.buffer || '') + data;

                if (session.buffer.length > maxBufferSize) {
                    session.buffer = session.buffer.slice(-maxBufferSize);
                }

                this.sessions.set(sessionId, session);
            }
        } else {
            console.warn(`[sessionManager] 尝试向不存在的会话 ${sessionId} 添加数据`);
        }
    }
    
    
    // 清除缓冲区
    clearBuffer(sessionId) {
        if (this.sessions.has(sessionId)) {
            const session = this.sessions.get(sessionId);
            session.buffer = '';
            this.sessions.set(sessionId, session);
        } else {
            console.warn(`[sessionManager] 尝试清除不存在的会话 ${sessionId} 的缓冲区`);
        }
    }

    // 获取会话的远程工作目录
    getRemotePath(sessionId) {
        const session = this.sessions.get(sessionId);
        return session ? session.currentRemotePath || '/' : '/';
    }

    // 更新会话的远程工作目录
    updateRemotePath(sessionId, path) {
        if (this.sessions.has(sessionId)) {
            const session = this.sessions.get(sessionId);
            session.currentRemotePath = path;
            this.sessions.set(sessionId, session);
        }
    }

}

// 导出单例实例
const sessionManager = new SessionManager();
export default sessionManager;