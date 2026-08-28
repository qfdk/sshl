export function FileManager() {
  return (
    <div className="file-manager">
      <div id="file-manager-loading" className="loading-overlay hidden">
        <div className="spinner" />
        <div className="loading-text">正在加载文件...</div>
      </div>

      <div className="split-pane">
        <div className="pane local-pane">
          <div className="pane-header">
            <span>本地文件</span>
            <div className="pane-actions">
              <button id="local-home" className="icon-button" title="主目录">
                <i data-lucide="house" data-size="12" />
              </button>
              <button id="local-refresh" className="icon-button" title="刷新">
                <i data-lucide="refresh-cw" data-size="12" />
              </button>
            </div>
          </div>
          <div className="path-bar">
            <input type="text" id="local-path" className="path-input" autoComplete="off" autoCapitalize="off" autoCorrect="off" spellCheck={false} />
            <button id="browse-local" className="icon-button">
              <i data-lucide="folder-open" data-size="12" />
            </button>
          </div>
          <div className="file-list-container">
            <table className="file-list" id="local-files">
              <thead>
                <tr>
                  <th>名称</th>
                  <th>大小</th>
                  <th>修改日期</th>
                </tr>
              </thead>
              <tbody />
            </table>
          </div>
        </div>

        <div className="pane-divider" id="pane-divider" title="拖动调整面板宽度" />

        <div className="pane remote-pane">
          <div className="pane-header">
            <span>远程文件</span>
            <div className="pane-actions">
              <button id="remote-home" className="icon-button" title="主目录">
                <i data-lucide="house" data-size="12" />
              </button>
              <button id="remote-refresh" className="icon-button" title="刷新">
                <i data-lucide="refresh-cw" data-size="12" />
              </button>
            </div>
          </div>
          <div className="path-bar">
            <input type="text" id="remote-path" className="path-input" defaultValue="/" autoComplete="off" autoCapitalize="off" autoCorrect="off" spellCheck={false} />
            <button id="go-remote-path" className="icon-button">
              <i data-lucide="chevron-right" data-size="12" />
            </button>
          </div>
          <div className="file-list-container">
            <table className="file-list" id="remote-files">
              <thead>
                <tr>
                  <th>名称</th>
                  <th>大小</th>
                  <th>修改日期</th>
                  <th>所有者</th>
                  <th>权限</th>
                </tr>
              </thead>
              <tbody />
            </table>
          </div>
        </div>
      </div>

      <div className="transfer-status">
        <div className="transfer-progress">
          <div className="progress-bar" id="transfer-progress-bar" />
        </div>
        <div className="transfer-info" id="transfer-info" />
      </div>
    </div>
  );
}
