import { useEffect } from 'react';
import { initializeApp } from '../../assets/js/main-entry.js';
import { Sidebar } from './layout/Sidebar';
import { Terminal } from './layout/Terminal';
import { FileManager } from './layout/FileManager';
import { ConnectionDialog } from './layout/ConnectionDialog';
import { SettingsDialog } from './layout/SettingsDialog';

export function App() {
  useEffect(() => {
    initializeApp();
  }, []);

  return (
    <>
      <div className="app-container">
        <Sidebar />

        <div className="main-content">
          <div className="tabs">
            <div className="tabs-left">
              <div className="tab active" data-tab="terminal">终端</div>
              <div className="tab" data-tab="file-manager">文件管理</div>
            </div>
            <div className="tabs-right">
              <div className="fill-password-group" id="fill-password-group" hidden>
                <button id="fill-password-btn" className="fill-password-btn" type="button" title="填充密码（用于 sudo / su 等密码提示）">
                  <span className="fill-password-icon" />
                  <span className="fill-password-label">填充密码</span>
                </button>
                <button id="fill-password-menu-btn" className="fill-password-caret" type="button" title="选择账号密码 / 管理" aria-label="选择账号密码">▾</button>
                <div id="fill-password-menu" className="fill-password-menu" hidden />
              </div>
              <div className="server-info" id="main-server-info">
                <span className="server-indicator" />
                <span className="server-name">未连接</span>
              </div>
            </div>
          </div>

          <div className="tab-content">
            <div className="tab-pane active" id="terminal-tab">
              <Terminal />
            </div>
            <div className="tab-pane" id="file-manager-tab">
              <FileManager />
            </div>
          </div>
        </div>
      </div>

      <div className="status-bar">
        <div className="connection-status">
          <span className="status-indicator offline" />
          <span className="status-text">未连接</span>
        </div>
      </div>
      <ConnectionDialog />
      <SettingsDialog />
    </>
  );
}
