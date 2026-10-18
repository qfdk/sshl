import { useEffect, useRef, useSyncExternalStore } from 'react';
import { getCurrentSessionId, subscribe } from '../lib/app-state';
import { useTerminal } from '../features/terminal/useTerminal';
import sessionManager from '../lib/session-manager';

const subscribeCurrentSessionId = (callback: () => void) => subscribe('currentSessionId', callback);

export function Terminal() {
  const containerRef = useRef<HTMLDivElement>(null);
  const terminalManager = useTerminal(containerRef);
  const sessionId = useSyncExternalStore(subscribeCurrentSessionId, getCurrentSessionId, () => null);

  useEffect(() => {
    (window as Window & { terminalManager?: typeof terminalManager }).terminalManager = terminalManager;
    (window as Window & { sessionManager?: typeof sessionManager }).sessionManager = sessionManager;
  }, [terminalManager]);

  // 终端是浅色窗口里的一张深色卡片；卡片底色仍跟随 --term-bg（OSC 11 / 边缘采样），
  // 与 xterm 的 padding 保持同色。
  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="relative min-h-0 flex-1 overflow-hidden rounded-[14px] shadow-[0_1px_2px_rgba(22,24,29,0.10),0_8px_24px_rgba(22,24,29,0.10)]" style={{ backgroundColor: 'var(--term-bg, #1d1d1d)' }}>
        <div ref={containerRef} id="terminal-container" className="terminal-container h-full w-full" />
        <div id="terminal-placeholder" className={`absolute inset-0 grid place-items-center bg-[#1d1d1d] ${sessionId ? 'hidden' : ''}`}>
          <div className="text-sm text-slate-300">点击左侧连接或创建新连接开始使用</div>
        </div>
      </div>
    </div>
  );
}
