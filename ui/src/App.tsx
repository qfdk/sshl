import { useSyncExternalStore } from 'react';
import { getActiveTabId, setActiveTabId, subscribe } from '../../assets/js/app-state.mjs';
import { Tabs, TabsContent, TabsList, TabsTrigger } from './components/ui/tabs';
import { Sidebar } from './layout/Sidebar';
import { Terminal } from './layout/Terminal';
import { FileManager } from './layout/FileManager';
import { ConnectionDialog } from './layout/ConnectionDialog';
import { SettingsDialog } from './layout/SettingsDialog';

// useSyncExternalStore 要求 subscribe 引用稳定：内联箭头函数会让 React 每次渲染都
// 重新订阅，通知会在 unsubscribe/subscribe 的间隙丢失。丢一次之后 store 已是新值，
// setter 的「值未变化直接 return」会让后续调用全部短路，UI 再也不更新。
const subscribeActiveTabId = (callback: () => void) => subscribe('activeTabId', callback);

export function App() {
  const activeTabId = useSyncExternalStore(
    subscribeActiveTabId,
    getActiveTabId,
    () => 'terminal',
  );



  return (
    <>
      <div className="app-container">
        <Sidebar />

        <div className="main-content">
          <Tabs value={activeTabId} onValueChange={setActiveTabId} className="tabs">
            <div className="tabs-header">
              <div className="tabs-left">
                <TabsList className="h-auto justify-start rounded-none bg-transparent p-0 text-foreground">
                  <TabsTrigger value="terminal" data-tab="terminal" className="tab">
                    终端
                  </TabsTrigger>
                  <TabsTrigger value="file-manager" data-tab="file-manager" className="tab">
                    文件管理
                  </TabsTrigger>
                </TabsList>
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
              <TabsContent value="terminal" id="terminal-tab" forceMount className="tab-pane mt-0 data-[state=inactive]:hidden">
                <Terminal />
              </TabsContent>
              <TabsContent value="file-manager" id="file-manager-tab" forceMount className="tab-pane mt-0 data-[state=inactive]:hidden">
                <FileManager />
              </TabsContent>
            </div>
          </Tabs>
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
