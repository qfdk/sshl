export function Sidebar() {
  return (
    <div className="sidebar" id="sidebar">
      <div className="sidebar-header">
        <div className="app-logo">SSHL</div>
        <button id="toggle-sidebar" className="icon-button">
          <i data-lucide="chevron-left" data-size="16" />
        </button>
      </div>

      <button id="new-connection-btn" className="primary-button">
        <i data-lucide="plus" data-size="18" data-stroke="2.75" />
        <span>新建连接</span>
      </button>

      <div className="search-box">
        <input type="text" id="connection-search" placeholder="搜索连接..." autoComplete="off" autoCapitalize="off" autoCorrect="off" spellCheck={false} />
      </div>

      <div className="connection-list" id="connection-list" />

      <div className="sidebar-footer">
        <button id="open-settings-btn" className="icon-button sidebar-footer-btn" title="设置">
          <i data-lucide="settings" data-size="16" />
          <span>设置</span>
        </button>
      </div>
    </div>
  );
}
