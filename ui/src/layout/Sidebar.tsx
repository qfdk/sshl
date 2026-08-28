import { ChevronLeft, Plus, Settings } from 'lucide-react';
import { Button } from '../components/ui/button';
import { Input } from '../components/ui/input';

export function Sidebar() {
  return (
    <div className="sidebar" id="sidebar">
      <div className="sidebar-header">
        <div className="app-logo">SSHL</div>
        <Button id="toggle-sidebar" variant="ghost" size="icon" className="icon-button">
          <ChevronLeft className="h-4 w-4" />
        </Button>
      </div>

      <Button id="new-connection-btn" className="primary-button">
        <Plus className="h-4 w-4" strokeWidth={2.75} />
        <span>新建连接</span>
      </Button>

      <div className="search-box">
        <Input type="text" id="connection-search" placeholder="搜索连接..." autoComplete="off" autoCapitalize="off" autoCorrect="off" spellCheck={false} />
      </div>

      <div className="connection-list" id="connection-list" />

      <div className="sidebar-footer">
        <Button id="open-settings-btn" variant="ghost" className="icon-button sidebar-footer-btn" title="设置">
          <Settings className="h-4 w-4" />
          <span>设置</span>
        </Button>
      </div>
    </div>
  );
}
