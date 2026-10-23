import test from 'node:test';
import assert from 'node:assert/strict';
import { TAB_HINT_DELAY_MS, createCommandHint, tabForShortcut } from '../ui/src/lib/tab-shortcuts.ts';

const press = (code, overrides = {}) => ({ code, key: '', metaKey: true, ctrlKey: false, shiftKey: false, altKey: false, ...overrides });

test('⌘1 终端、⌘2 文件管理；按物理键位，AZERTY 上数字键打出 & 也认', () => {
  assert.equal(tabForShortcut(press('Digit1')), 'terminal');
  assert.equal(tabForShortcut(press('Digit2')), 'file-manager');
  assert.equal(tabForShortcut(press('Digit1', { key: '&' })), 'terminal');
});

test('不带 ⌘、或带 Shift/Option/Ctrl、或其他数字，都不切换', () => {
  assert.equal(tabForShortcut(press('Digit1', { metaKey: false })), null);
  assert.equal(tabForShortcut(press('Digit1', { shiftKey: true })), null);
  assert.equal(tabForShortcut(press('Digit1', { altKey: true })), null);
  assert.equal(tabForShortcut(press('Digit1', { ctrlKey: true })), null);
  assert.equal(tabForShortcut(press('Digit3')), null);
});

// 手动推进的计时器
function fakeTimers() {
  let pending = null;
  return {
    timers: { set: (fn) => { pending = fn; return 1; }, clear: () => { pending = null; } },
    fire: () => { const fn = pending; pending = null; fn?.(); },
    get armed() { return pending !== null; },
  };
}

const meta = (overrides = {}) => ({ key: 'Meta', metaKey: true, shiftKey: false, altKey: false, ctrlKey: false, ...overrides });

test('按住 ⌘ 满延时才浮出，松开收起', () => {
  const seen = [];
  const clock = fakeTimers();
  const hint = createCommandHint(visible => seen.push(visible), clock.timers);
  hint.keydown(meta());
  assert.deepEqual(seen, [], '刚按下还不显示');
  assert.ok(clock.armed);
  clock.fire();
  assert.deepEqual(seen, [true]);
  hint.keyup({ key: 'Meta' });
  assert.deepEqual(seen, [true, false]);
  assert.ok(TAB_HINT_DELAY_MS > 0 && TAB_HINT_DELAY_MS <= 400);
});

test('⌘C 这类快速组合键不闪出角标；已浮出时按 ⌘2 保持显示', () => {
  const seen = [];
  const clock = fakeTimers();
  const hint = createCommandHint(visible => seen.push(visible), clock.timers);
  hint.keydown(meta());
  hint.keydown({ key: 'c', metaKey: true });
  assert.equal(clock.armed, false, '按了别的键，取消待浮出');
  hint.keyup({ key: 'Meta' });
  assert.deepEqual(seen, []);

  hint.keydown(meta());
  clock.fire();
  hint.keydown({ key: '2', metaKey: true });
  assert.deepEqual(seen, [true], '正在用数字切换，角标不收');
});

test('切走窗口、或漏掉了 ⌘ 的 keyup，角标都会收起', () => {
  const seen = [];
  const clock = fakeTimers();
  const hint = createCommandHint(visible => seen.push(visible), clock.timers);
  hint.keydown(meta());
  clock.fire();
  hint.reset();
  assert.deepEqual(seen, [true, false]);

  hint.keydown(meta());
  clock.fire();
  // ⌘ 已松开但 keyup 没收到（如松开时焦点在别处）：下一次不带 ⌘ 的按键收起
  hint.keydown({ key: 'x', metaKey: false });
  assert.deepEqual(seen, [true, false, true, false]);
});

test('默认计时器在 WebKit 下也能用：setTimeout 不能以对象方法调用（否则 Illegal invocation，角标永不浮出）', async () => {
  const realSetTimeout = globalThis.setTimeout;
  // 模拟 WebKit：this 不是 window（这里是 globalThis）时抛错
  globalThis.setTimeout = function (fn, ms) {
    if (this !== undefined && this !== globalThis) throw new TypeError("Can only call Window.setTimeout on instances of Window");
    return realSetTimeout(fn, ms);
  };
  try {
    const seen = [];
    const hint = createCommandHint(visible => seen.push(visible));
    hint.keydown(meta());
    await new Promise(resolve => realSetTimeout(resolve, TAB_HINT_DELAY_MS + 50));
    assert.deepEqual(seen, [true]);
    hint.reset();
  } finally {
    globalThis.setTimeout = realSetTimeout;
  }
});
