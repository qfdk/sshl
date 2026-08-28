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

  return (
    <div className="flex h-full min-h-0 flex-col" style={{ backgroundColor: 'var(--term-bg, #1d1d1d)' }}>
      <div className="relative min-h-0 flex-1 overflow-hidden">
        <div ref={containerRef} id="terminal-container" className="h-full w-full" />
        <div id="terminal-placeholder" className={`absolute inset-0 grid place-items-center bg-[#1d1d1d] ${sessionId ? 'hidden' : ''}`}>
          <div className="text-sm text-slate-300">点击左侧连接或创建新连接开始使用</div>
        </div>
      </div>
    </div>
  );
}
