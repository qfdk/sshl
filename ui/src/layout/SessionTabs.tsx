import { Plus, X } from 'lucide-react';
import { useSyncExternalStore } from 'react';
import { getCurrentSessionId, setConnectionDialogOpen, setEditingConnection, subscribe } from '../lib/app-state';
import sessionManager from '../lib/session-manager';
import { toSessionTabs } from '../lib/session-tabs';
import { cn } from '../lib/utils';
import type { ConnectionManager } from '../features/connections/useConnections';

const subscribeCurrentSessionId = (callback: () => void) => subscribe('currentSessionId', callback);

export function SessionTabs({ manager }: { manager: ConnectionManager }) {
  const currentSessionId = useSyncExternalStore(subscribeCurrentSessionId, getCurrentSessionId, () => null);
  // sessionManager 不是响应式的；会话增删都会 bump manager.revision，App 随之重渲染，这里跟着读到最新。
  const tabs = toSessionTabs(sessionManager.getAllSessions(), manager.connections, currentSessionId);
  return (
    <div data-tauri-drag-region role="tablist" aria-label="已打开的会话" className="flex min-w-0 flex-wrap items-center gap-1.5">
      {tabs.map(tab => (
        <div key={tab.sessionId} className={cn('flex h-[34px] items-center rounded-[10px] border', tab.active ? 'border-border bg-card shadow-sm' : 'border-transparent hover:bg-foreground/[0.04]')}>
          {/* 走 connectToSaved 而不是 switchToSession：会话已失效时它会接着重连，和侧边栏双击一致 */}
          <button type="button" role="tab" aria-selected={tab.active} className="flex h-full min-w-0 items-center gap-2 pl-3 pr-1.5 text-sm" onClick={() => { if (!tab.active) void manager.connectToSaved(tab.connectionId); }}>
            <span className="h-[7px] w-[7px] shrink-0 rounded-full bg-success" />
            <span className={cn('truncate', tab.active ? 'font-semibold' : 'text-foreground/80')}>{tab.name}</span>
            {tab.active && tab.host && <span className="truncate font-mono text-[11px] text-muted-foreground">{tab.host}</span>}
          </button>
          <button type="button" className="mr-1 grid h-[26px] w-[26px] shrink-0 place-items-center rounded-[7px] text-muted-foreground hover:bg-foreground/[0.06] hover:text-foreground" title={`断开 ${tab.name}`} aria-label={`断开 ${tab.name}`} onClick={() => void manager.disconnect(tab.sessionId)}>
            <X className="h-3 w-3" />
          </button>
        </div>
      ))}
      {!tabs.length && <span data-tauri-drag-region className="px-1 text-sm text-muted-foreground">未连接</span>}
      <button type="button" className="grid h-[34px] w-[34px] place-items-center rounded-[10px] border border-dashed border-foreground/20 text-muted-foreground hover:border-foreground/35 hover:text-foreground" title="新建连接" aria-label="新建连接" onClick={() => { setEditingConnection(null); setConnectionDialogOpen(true); }}>
        <Plus className="h-3.5 w-3.5" />
      </button>
    </div>
  );
}
