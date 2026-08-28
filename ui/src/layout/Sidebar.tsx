import { ChevronLeft, ChevronRight, Plus, Settings } from 'lucide-react';
import { useState } from 'react';
import { setConnectionDialogOpen, setEditingConnection, setSettingsDialogOpen } from '../lib/app-state';
import { Button } from '../components/ui/button';
import { Input } from '../components/ui/input';
import { ConnectionList } from '../features/connections/ConnectionList';
import type { ConnectionManager } from '../features/connections/useConnections';

export function Sidebar({ manager }: { manager: ConnectionManager }) {
  const [collapsed, setCollapsed] = useState(false);
  const [query, setQuery] = useState('');
  return (
    <aside className={`flex h-full min-h-0 shrink-0 select-none flex-col overflow-hidden border-r border-border bg-card transition-[width] duration-200 ${collapsed ? 'w-16' : 'w-72'}`} id="sidebar" data-collapsed={collapsed}>
      <div className={`flex h-14 shrink-0 items-center border-b border-border ${collapsed ? 'justify-center px-1' : 'justify-between px-3'}`}>
        {!collapsed && <div className="font-semibold tracking-wide">SSHL</div>}
        <Button id="toggle-sidebar" variant="ghost" size="icon" title={collapsed ? '展开侧边栏' : '收起侧边栏'} className={collapsed ? '' : 'ml-auto'} onClick={() => setCollapsed(value => !value)}>{collapsed ? <ChevronRight className="h-4 w-4" /> : <ChevronLeft className="h-4 w-4" />}</Button>
      </div>
      <div className={collapsed ? 'flex shrink-0 justify-center pt-2' : 'contents'}>
        <Button
          id="new-connection-btn"
          size={collapsed ? 'icon' : 'default'}
          className={collapsed ? '' : 'm-3'}
          title={collapsed ? '新建连接' : undefined}
          onClick={() => { setEditingConnection(null); setConnectionDialogOpen(true); }}
        >
          <Plus className="h-4 w-4" />{!collapsed && <span>新建连接</span>}
        </Button>
      </div>
      {!collapsed && <div className="px-3 pb-2"><Input type="text" id="connection-search" placeholder="搜索连接..." value={query} onChange={event => setQuery(event.target.value)} autoComplete="off" autoCapitalize="off" autoCorrect="off" spellCheck={false} /></div>}
      <div className="mt-2 min-h-0 flex-1" id="connection-list"><ConnectionList manager={manager} query={collapsed ? '' : query} collapsed={collapsed} /></div>
      <div className={`mt-auto shrink-0 border-t border-border ${collapsed ? 'flex justify-center p-2' : 'p-2'}`}><Button id="open-settings-btn" variant="ghost" size={collapsed ? 'icon' : 'default'} className={collapsed ? '' : 'w-full justify-start gap-2'} title="设置" onClick={() => setSettingsDialogOpen(true)}><Settings className="h-4 w-4" />{!collapsed && <span>设置</span>}</Button></div>
    </aside>
  );
}
