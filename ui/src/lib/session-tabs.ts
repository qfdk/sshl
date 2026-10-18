type OpenSession = { id: string; connectionId: string; name?: unknown };
type SavedConnection = { id: string; name?: string; host?: string };

export type SessionTab = {
  sessionId: string;
  connectionId: string;
  name: string;
  host: string;
  active: boolean;
};

// 顶栏会话标签：每个已打开的会话一个，按打开顺序排列。名称优先取保存的连接（可能被改过名），
// 连接已不在列表里时退回会话建立时记下的名称。
export function toSessionTabs(sessions: OpenSession[], connections: SavedConnection[], currentSessionId: string | null): SessionTab[] {
  const byId = new Map(connections.map(connection => [connection.id, connection]));
  return sessions.map(session => {
    const connection = byId.get(session.connectionId);
    return {
      sessionId: session.id,
      connectionId: session.connectionId,
      name: connection?.name || String(session.name || ''),
      host: connection?.host || '',
      active: session.id === currentSessionId,
    };
  });
}
