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
    <aside className={`flex h-full min-h-0 w-72 shrink-0 flex-col border-r border-border bg-card transition-[width] ${collapsed ? 'w-14' : ''}`} id="sidebar" data-collapsed={collapsed}>
      <div className="flex h-14 items-center justify-between border-b border-border px-3">
        {!collapsed && <div className="font-semibold tracking-wide">SSHL</div>}
        <Button id="toggle-sidebar" variant="ghost" size="icon" className="ml-auto" onClick={() => setCollapsed(value => !value)}>{collapsed ? <ChevronRight className="h-4 w-4" /> : <ChevronLeft className="h-4 w-4" />}</Button>
      </div>
      {!collapsed && <>
        <Button id="new-connection-btn" className="m-3" onClick={() => { setEditingConnection(null); setConnectionDialogOpen(true); }}><Plus className="h-4 w-4" /><span>新建连接</span></Button>
        <div className="px-3 pb-2"><Input type="text" id="connection-search" placeholder="搜索连接..." value={query} onChange={event => setQuery(event.target.value)} autoComplete="off" autoCapitalize="off" autoCorrect="off" spellCheck={false} /></div>
        <div className="min-h-0 flex-1" id="connection-list"><ConnectionList manager={manager} query={query} /></div>
      </>}
      <div className="mt-auto border-t border-border p-2"><Button id="open-settings-btn" variant="ghost" className="w-full justify-start gap-2" title="设置" onClick={() => setSettingsDialogOpen(true)}><Settings className="h-4 w-4" />{!collapsed && <span>设置</span>}</Button></div>
    </aside>
  );
}
