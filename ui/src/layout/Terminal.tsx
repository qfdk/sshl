import { useEffect, useRef, useSyncExternalStore } from 'react';
import { initializeApp } from '../../../assets/js/main-entry.js';
import { getCurrentSessionId, subscribe } from '../../../assets/js/app-state.mjs';
import { useTerminal } from '../features/terminal/useTerminal';
import sessionManager from '../lib/session-manager';

const subscribeCurrentSessionId = (callback: () => void) => subscribe('currentSessionId', callback);

export function Terminal() {
  const containerRef = useRef<HTMLDivElement>(null);
  const terminalManager = useTerminal(containerRef);
  const sessionId = useSyncExternalStore(subscribeCurrentSessionId, getCurrentSessionId, () => null);

  useEffect(() => {
    (window as Window & { terminalManager?: typeof terminalManager }).terminalManager = terminalManager;
    initializeApp(terminalManager, sessionManager);
  }, [terminalManager]);

  return (
    <div className="terminal-view">
      <div className="terminal-content">
        <div ref={containerRef} id="terminal-container" className="terminal-container" />
        <div id="terminal-placeholder" className={`terminal-placeholder ${sessionId ? 'hidden' : ''}`}>
          <div className="placeholder-content">
            <p>点击左侧连接或创建新连接开始使用</p>
          </div>
        </div>
      </div>
    </div>
  );
}
