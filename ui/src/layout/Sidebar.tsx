import { ChevronLeft, ChevronRight, Plus, Search, Settings, Terminal } from 'lucide-react';
import { useState } from 'react';
import { setConnectionDialogOpen, setEditingConnection, setSettingsDialogOpen } from '../lib/app-state';
import { Button } from '../components/ui/button';
import { Input } from '../components/ui/input';
import { ConnectionList } from '../features/connections/ConnectionList';
import type { ConnectionManager } from '../features/connections/useConnections';

export function Sidebar({ manager }: { manager: ConnectionManager }) {
  const [collapsed, setCollapsed] = useState(false);
  const [query, setQuery] = useState('');
  const openNewConnection = () => { setEditingConnection(null); setConnectionDialogOpen(true); };
  return (
    <aside className={`flex h-full min-h-0 shrink-0 select-none flex-col overflow-hidden transition-[width] duration-200 ${collapsed ? 'w-16' : 'w-52'}`} id="sidebar" data-collapsed={collapsed}>
      <div className={`flex h-14 shrink-0 items-center ${collapsed ? 'justify-center px-1' : 'gap-2 pl-4 pr-2'}`}>
        {!collapsed && (
          <>
            <span className="flex h-[22px] w-[22px] items-center justify-center rounded-md bg-foreground" aria-hidden="true"><Terminal className="h-3.5 w-3.5 text-[#5695d7]" /></span>
            <span className="font-semibold tracking-wide">SSHL</span>
          </>
        )}
        <Button id="toggle-sidebar" variant="ghost" size="icon" title={collapsed ? '展开侧边栏' : '收起侧边栏'} className={`h-9 w-9 ${collapsed ? '' : 'ml-auto'}`} onClick={() => setCollapsed(value => !value)}>{collapsed ? <ChevronRight className="h-4 w-4" /> : <ChevronLeft className="h-4 w-4" />}</Button>
      </div>
      {!collapsed && (
        <div className="relative px-3 pb-2">
          <Search className="pointer-events-none absolute left-6 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" aria-hidden="true" />
          <Input type="text" id="connection-search" className="h-9 bg-card pl-9" placeholder="搜索连接..." aria-label="搜索连接" value={query} onChange={event => setQuery(event.target.value)} autoComplete="off" autoCapitalize="off" autoCorrect="off" spellCheck={false} />
        </div>
      )}
      <div className="mt-1 min-h-0 flex-1" id="connection-list"><ConnectionList manager={manager} query={collapsed ? '' : query} collapsed={collapsed} /></div>
      <div className={`mt-auto flex shrink-0 gap-2 ${collapsed ? 'flex-col items-center p-2' : 'p-3'}`}>
        <Button id="new-connection-btn" size={collapsed ? 'icon' : 'default'} className={collapsed ? 'h-9 w-9' : 'h-9 flex-1'} title={collapsed ? '新建连接' : undefined} onClick={openNewConnection}>
          <Plus className="h-4 w-4" />{!collapsed && <span>新建连接</span>}
        </Button>
        <Button id="open-settings-btn" variant="outline" size="icon" className="h-9 w-9 shrink-0 bg-card" title="设置" onClick={() => setSettingsDialogOpen(true)}><Settings className="h-4 w-4" /></Button>
      </div>
    </aside>
  );
}
