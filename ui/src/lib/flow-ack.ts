// 终端输出流控的前端半边。后端把终端通道开在 russh 手动窗口模式（src-tauri/vendor/README.md）：
// 服务端最多比这里确认过的多发 2MB，xterm 处理不过来时远端进程被背压挡住，而不是把积压
// 堆进 WebView / xterm（xterm 超 50MB 直接丢数据），^C 之后也不用再等积压滚完。
//
// 每个 ssh:data 带 bytes（对应的原始 SSH 字节数，激活快照为 0）。xterm 解析完（write 回调；
// 它走 setTimeout，forceMount 隐藏的标签页照常触发）才算确认。按会话攒到 ACK_THRESHOLD
// 才发一次 IPC：打字、vim 重绘这种小流量一次都不发；攒着的零头远小于 2MB 窗口，不会卡住。

export const ACK_THRESHOLD = 128 * 1024;
/** 让后端把已发出、未确认的全部算作确认（后端按实际发出量封顶，多确认不会撑大窗口）。 */
export const ACK_ALL = 0xffffffff;

type Writable = { write(data: string, callback?: () => void): void };
type DataEvent = { sessionId: string; data: string; bytes?: number };
type SessionFlow = { pending: number; outstanding: number; generation: number };

export function createFlowAck(send: (sessionId: string, bytes: number) => void, threshold = ACK_THRESHOLD) {
  const sessions = new Map<string, SessionFlow>();

  const flowFor = (sessionId: string) => {
    let flow = sessions.get(sessionId);
    if (!flow) {
      flow = { pending: 0, outstanding: 0, generation: 0 };
      sessions.set(sessionId, flow);
    }
    return flow;
  };

  const consume = (sessionId: string, flow: SessionFlow, bytes: number) => {
    flow.pending += bytes;
    if (flow.pending < threshold) return;
    const n = flow.pending;
    flow.pending = 0;
    send(sessionId, n);
  };

  return {
    /** 把一条 ssh:data 写进终端，解析完才确认；没有终端、或写入抛错（数据已被丢弃）立即确认。 */
    deliver(terminal: Writable | null | undefined, event: DataEvent) {
      const { sessionId } = event;
      const bytes = event.bytes || 0;
      if (!terminal) {
        if (bytes) consume(sessionId, flowFor(sessionId), bytes);
        return;
      }
      if (!bytes) {
        terminal.write(event.data);
        return;
      }
      const flow = flowFor(sessionId);
      const generation = flow.generation;
      flow.outstanding += bytes;
      let settled = false;
      const done = () => {
        if (settled) return;
        settled = true;
        // 终端已销毁 / 已重同步：这份已经一起确认过了
        if (sessions.get(sessionId) !== flow || flow.generation !== generation) return;
        flow.outstanding -= bytes;
        consume(sessionId, flow, bytes);
      };
      try {
        terminal.write(event.data, done);
      } catch (error) {
        done();
        throw error;
      }
    },

    /** 终端被销毁：写进去但可能永远等不到回调的字节一并确认，旧回调作废。 */
    terminalGone(sessionId: string) {
      const flow = sessions.get(sessionId);
      if (!flow) return;
      flow.generation++;
      const bytes = flow.outstanding;
      flow.outstanding = 0;
      if (bytes) consume(sessionId, flow, bytes);
    },

    /** ssh:data 监听（重新）注册：之前可能有事件没人收、永远不会确认，让后端全部补回，旧回调作废。 */
    resync(sessionId: string) {
      const flow = flowFor(sessionId);
      flow.generation++;
      flow.outstanding = 0;
      flow.pending = 0;
      send(sessionId, ACK_ALL);
    },

    forget(sessionId: string) {
      sessions.delete(sessionId);
    },
  };
}

export const flowAck = createFlowAck((sessionId, bytes) => {
  void window.api.ssh.ackData(sessionId, bytes);
});
