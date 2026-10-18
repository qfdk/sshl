type ProbeResult = { success?: boolean; alive?: boolean } | null | undefined;

type Deps = {
  validate: (sessionId: string) => Promise<ProbeResult>;
  getCurrentSessionId: () => string | null;
  discard: (sessionId: string, connectionId: string) => void;
  reconnect: (connectionId: string) => void;
};

// 后台探活：切换会话不再先 await 探活（开关一个 probe 通道，≥1 RTT，最坏 2s 超时），
// 先切过去，失效再善后。探活中的会话记在 pending 里；用户在探活期间主动断开会 forget 它，
// 借此区分「连接失效该重连」和「用户自己断开」。
export function createSessionRevalidator({ validate, getCurrentSessionId, discard, reconnect }: Deps) {
  const pending = new Map<string, string>();

  const settle = (sessionId: string, connectionId: string, result: ProbeResult) => {
    if (!pending.delete(sessionId)) return;
    if (result?.success && result.alive) return;
    // 后端探活失败时已关闭该会话；ssh:closed 可能先到并把 current 清成 null，仍算用户停留在此。
    const current = getCurrentSessionId();
    const stillViewing = current === sessionId || current === null;
    discard(sessionId, connectionId);
    if (stillViewing) reconnect(connectionId);
  };

  const revalidate = (sessionId: string, connectionId: string) => {
    if (pending.has(sessionId)) return undefined;
    pending.set(sessionId, connectionId);
    return validate(sessionId).then(
      result => settle(sessionId, connectionId, result),
      () => settle(sessionId, connectionId, null),
    );
  };

  const forget = (sessionId: string) => {
    pending.delete(sessionId);
  };

  return { revalidate, forget };
}
