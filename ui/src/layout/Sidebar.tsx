import { ChevronLeft, ChevronRight, Plus, Search, Settings } from 'lucide-react';
import { useState } from 'react';
import appIcon from '../../../icon.png';
import { setConnectionDialogOpen, setEditingConnection, setSettingsDialogOpen } from '../lib/app-state';
import { Button } from '../components/ui/button';
import { Input } from '../components/ui/input';
import { ConnectionList } from '../features/connections/ConnectionList';
import type { ConnectionManager } from '../features/connections/useConnections';

export function Sidebar({ manager }: { manager: ConnectionManager }) {
  const [collapsed, setCollapsed] = useState(false);
  const [query, setQuery] = useState('');
  const openNewConnection = () => { setEditingConnection(null); setConnectionDialogOpen(true); };
  const toggle = (
    <Button id="toggle-sidebar" variant="ghost" size="icon" title={collapsed ? '展开侧边栏' : '收起侧边栏'} className={`h-9 w-9 ${collapsed ? '' : 'ml-auto'}`} onClick={() => setCollapsed(value => !value)}>{collapsed ? <ChevronRight className="h-4 w-4" /> : <ChevronLeft className="h-4 w-4" />}</Button>
  );
  return (
    <aside className={`flex h-full min-h-0 shrink-0 select-none flex-col overflow-hidden transition-[width] duration-200 ${collapsed ? 'w-[76px]' : 'w-52'}`} id="sidebar" data-collapsed={collapsed}>
      {/* 标题栏是透明叠加的（tauri.conf.json：Overlay + trafficLightPosition）：红绿灯单独占这一行，
          与右侧顶栏同一中线；这一行只作窗口拖动区。收起时 76px 宽刚好容下红绿灯。 */}
      <div data-tauri-drag-region className="h-[50px] shrink-0" />
      {collapsed
        ? <div className="flex shrink-0 justify-center pb-1">{toggle}</div>
        : (
          <div data-tauri-drag-region className="flex h-11 shrink-0 items-center gap-2 pl-4 pr-2">
            {/* 与 app 图标同一张 icon.png；图里四周约 10% 透明边距，放大并用负边距抵掉，可见方块约 24px */}
            <img data-tauri-drag-region src={appIcon} alt="" aria-hidden="true" className="-m-[3px] h-[30px] w-[30px] shrink-0" draggable={false} />
            <span data-tauri-drag-region className="font-semibold tracking-wide">SSHL</span>
            {toggle}
          </div>
        )}
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
