import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { ACK_ALL, ACK_THRESHOLD, createFlowAck } from '../ui/src/lib/flow-ack.ts';

function setup() {
  const sent = [];
  const flow = createFlowAck((sessionId, bytes) => sent.push([sessionId, bytes]));
  // 模拟 xterm：write 回调在「解析完」时才触发，这里由测试手动 parse()
  const terminal = (opts = {}) => {
    const queue = [];
    return {
      written: [],
      write(data, callback) {
        if (opts.throws) throw new Error('write data discarded, use flow control to avoid losing data');
        this.written.push(data);
        queue.push(callback);
      },
      parse(n = queue.length) { for (const cb of queue.splice(0, n)) cb?.(); },
    };
  };
  return { sent, flow, terminal };
}

const event = (bytes, sessionId = 's1') => ({ sessionId, data: 'x'.repeat(Math.min(bytes, 8)), bytes });

test('typing and vim-sized output never sends an ack', () => {
  const { sent, flow, terminal } = setup();
  const term = terminal();
  for (let i = 0; i < 500; i++) flow.deliver(term, event(200));
  term.parse();
  assert.deepEqual(sent, [], '100KB of interactive output must not cost an IPC round trip');
});

test('bytes are acked only after xterm has parsed them, batched per 128KB', () => {
  const { sent, flow, terminal } = setup();
  const term = terminal();
  for (let i = 0; i < 8; i++) flow.deliver(term, event(32 * 1024));
  assert.deepEqual(sent, [], 'nothing parsed yet');
  term.parse(4);
  assert.deepEqual(sent, [['s1', ACK_THRESHOLD]]);
  term.parse();
  assert.deepEqual(sent, [['s1', ACK_THRESHOLD], ['s1', ACK_THRESHOLD]]);
});

test('sessions are accounted separately', () => {
  const { sent, flow, terminal } = setup();
  const a = terminal();
  const b = terminal();
  flow.deliver(a, event(100 * 1024, 'a'));
  flow.deliver(b, event(100 * 1024, 'b'));
  a.parse();
  b.parse();
  assert.deepEqual(sent, []);
  flow.deliver(a, event(28 * 1024, 'a'));
  a.parse();
  assert.deepEqual(sent, [['a', ACK_THRESHOLD]]);
});

test('snapshot events (bytes 0) are written without an ack callback', () => {
  const { sent, flow } = setup();
  const calls = [];
  flow.deliver({ write: (data, callback) => calls.push([data, callback]) }, { sessionId: 's1', data: 'banner', bytes: 0 });
  assert.deepEqual(calls, [['banner', undefined]]);
  assert.deepEqual(sent, []);
});

test('output for a session without a terminal is acked right away', () => {
  const { sent, flow } = setup();
  flow.deliver(null, event(ACK_THRESHOLD));
  assert.deepEqual(sent, [['s1', ACK_THRESHOLD]]);
});

test('a write that throws (xterm discarded it) is acked and the error still surfaces', () => {
  const { sent, flow, terminal } = setup();
  assert.throws(() => flow.deliver(terminal({ throws: true }), event(ACK_THRESHOLD)), /discarded/);
  assert.deepEqual(sent, [['s1', ACK_THRESHOLD]]);
});

test('disposing a terminal acks what it never parsed, and its late callbacks are ignored', () => {
  const { sent, flow, terminal } = setup();
  const term = terminal();
  flow.deliver(term, event(100 * 1024));
  flow.deliver(term, event(28 * 1024));
  flow.terminalGone('s1');
  assert.deepEqual(sent, [['s1', ACK_THRESHOLD]]);
  term.parse(); // xterm 销毁后残留的 setTimeout 仍可能回调
  const next = terminal();
  flow.deliver(next, event(ACK_THRESHOLD));
  next.parse();
  assert.deepEqual(sent, [['s1', ACK_THRESHOLD], ['s1', ACK_THRESHOLD]], 'no double ack from the dead terminal');
});

test('resync asks the backend to settle everything and voids callbacks from before it', () => {
  const { sent, flow, terminal } = setup();
  const term = terminal();
  flow.deliver(term, event(100 * 1024));
  flow.resync('s1');
  assert.deepEqual(sent, [['s1', ACK_ALL]]);
  term.parse();
  flow.deliver(term, event(100 * 1024));
  term.parse();
  assert.deepEqual(sent, [['s1', ACK_ALL]], 'pre-resync bytes must not count again');
});

test('forget drops the session state', () => {
  const { sent, flow, terminal } = setup();
  const term = terminal();
  flow.deliver(term, event(100 * 1024));
  flow.forget('s1');
  term.parse();
  flow.deliver(term, event(100 * 1024));
  term.parse();
  assert.deepEqual(sent, []);
});

test('the app wires ssh:data, terminal disposal and session close through flow-ack', async () => {
  const connections = await readFile(new URL('../ui/src/features/connections/useConnections.ts', import.meta.url), 'utf8');
  const terminal = await readFile(new URL('../ui/src/features/terminal/useTerminal.ts', import.meta.url), 'utf8');
  const ipc = await readFile(new URL('../ui/src/lib/ipc.ts', import.meta.url), 'utf8');
  assert.match(connections, /onData\([\s\S]{0,300}flowAck\.deliver\(/, 'ssh:data must go through flowAck.deliver');
  assert.match(connections, /onData\([\s\S]{0,600}flowAck\.resync\(/, 'listener (re)registration must resync');
  assert.match(connections, /onClosed\([\s\S]{0,800}flowAck\.forget\(data\.sessionId\)/, 'ssh:closed must forget the session');
  assert.match(terminal, /const disposeTerminalInstance = [\s\S]{0,200}flowAck\.terminalGone\(sessionId\)/);
  assert.match(ipc, /ackData: \(sessionId: string, bytes: number\) => call\('ssh_ack_data'/);
});
