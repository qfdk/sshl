type OpenSession = { id: string; connectionId: string; name?: unknown };
type SavedConnection = { id: string; name?: string; host?: string };

export type SessionTab = {
  sessionId: string;
  connectionId: string;
  name: string;
  host: string;
  active: boolean;
};

// 按用户拖出来的顺序排标签：order 里有的按 order；之后新开的会话按打开顺序接在末尾；
// order 里已关闭的会话自然忽略。顺序只在本次运行内有效，会话本身就是临时的。
export function applyTabOrder<T extends { sessionId: string }>(tabs: T[], order: string[]): T[] {
  const rank = new Map(order.map((id, index) => [id, index]));
  return tabs
    .map((tab, index) => ({ tab, index, rank: rank.get(tab.sessionId) ?? Number.POSITIVE_INFINITY }))
    .sort((a, b) => a.rank - b.rank || a.index - b.index)
    .map(item => item.tab);
}

// 把拖动的标签放到目标标签的前或后；拖到自己身上或 id 不存在时原样返回。
export function moveTabId(ids: string[], dragged: string, target: string, side: 'before' | 'after'): string[] {
  if (dragged === target || !ids.includes(dragged) || !ids.includes(target)) return ids;
  const rest = ids.filter(id => id !== dragged);
  const at = rest.indexOf(target) + (side === 'after' ? 1 : 0);
  return [...rest.slice(0, at), dragged, ...rest.slice(at)];
}

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
