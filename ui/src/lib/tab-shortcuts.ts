import { useEffect, useRef, useState } from 'react';

export type MainTab = 'terminal' | 'file-manager';

// 按物理键位：AZERTY 等布局上数字键不按 Shift 打出的是 &é"'…，key 不是 '1'/'2'。
const TAB_BY_CODE: Record<string, MainTab> = {
  Digit1: 'terminal',
  Digit2: 'file-manager',
  Numpad1: 'terminal',
  Numpad2: 'file-manager',
};

/** 按住 ⌘ 多久后浮出数字角标：太短则 ⌘C/⌘V 这类快捷键会让角标一闪。 */
export const TAB_HINT_DELAY_MS = 250;

type ShortcutKey = Pick<KeyboardEvent, 'code' | 'metaKey' | 'ctrlKey' | 'shiftKey' | 'altKey'>;
type HintKey = Pick<KeyboardEvent, 'key' | 'metaKey'> & Partial<Pick<KeyboardEvent, 'shiftKey' | 'altKey' | 'ctrlKey'>>;
type Timers = { set: (fn: () => void, ms: number) => unknown; clear: (id: unknown) => void };

export function tabForShortcut(event: ShortcutKey): MainTab | null {
  if (!event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return null;
  return TAB_BY_CODE[event.code] ?? null;
}

// 默认计时器用箭头函数包一层：WebKit 里 setTimeout 以对象方法调用（timers.set(..)）会抛 Illegal invocation。
const browserTimers: Timers = {
  set: (fn, ms) => setTimeout(fn, ms),
  clear: id => clearTimeout(id as ReturnType<typeof setTimeout>),
};

/** 按住 ⌘ 时的数字角标：满延时浮出，松开 ⌘ / 失焦收起；延时内按了别的键（⌘C 等）就不浮出。 */
export function createCommandHint(onChange: (visible: boolean) => void, timers: Timers = browserTimers) {
  let timer: unknown = null;
  let visible = false;
  const show = (next: boolean) => {
    if (visible === next) return;
    visible = next;
    onChange(next);
  };
  const cancel = () => {
    if (timer === null) return;
    timers.clear(timer);
    timer = null;
  };
  return {
    keydown(event: HintKey) {
      if (event.key === 'Meta' && !event.shiftKey && !event.altKey && !event.ctrlKey) {
        if (!visible && timer === null) timer = timers.set(() => { timer = null; show(true); }, TAB_HINT_DELAY_MS);
        return;
      }
      if (!event.metaKey) {
        // ⌘ 已松开但没收到 keyup（松开时焦点不在本窗口等）
        cancel();
        show(false);
      } else if (!visible) {
        cancel();
      }
    },
    keyup(event: Pick<KeyboardEvent, 'key'>) {
      if (event.key !== 'Meta') return;
      cancel();
      show(false);
    },
    reset() {
      cancel();
      show(false);
    },
  };
}

/**
 * ⌘1 终端、⌘2 文件管理，按住 ⌘ 浮出数字角标。只在已连接服务器时生效（enabled）。
 * 捕获阶段监听：焦点在终端时，按键先到这里，拦下后 xterm 不会再处理。
 */
export function useTabShortcuts(enabled: boolean, setTab: (tab: MainTab) => void) {
  const [hintVisible, setHintVisible] = useState(false);
  const setTabRef = useRef(setTab);
  setTabRef.current = setTab;
  useEffect(() => {
    if (!enabled) return;
    const hint = createCommandHint(setHintVisible);
    const onKeyDown = (event: KeyboardEvent) => {
      const tab = tabForShortcut(event);
      if (tab) {
        event.preventDefault();
        event.stopPropagation();
        setTabRef.current(tab);
      }
      hint.keydown(event);
    };
    const onKeyUp = (event: KeyboardEvent) => hint.keyup(event);
    const reset = () => hint.reset();
    window.addEventListener('keydown', onKeyDown, true);
    window.addEventListener('keyup', onKeyUp, true);
    window.addEventListener('blur', reset);
    document.addEventListener('visibilitychange', reset);
    return () => {
      hint.reset();
      window.removeEventListener('keydown', onKeyDown, true);
      window.removeEventListener('keyup', onKeyUp, true);
      window.removeEventListener('blur', reset);
      document.removeEventListener('visibilitychange', reset);
    };
  }, [enabled]);
  return enabled && hintVisible;
}
