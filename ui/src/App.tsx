import { useEffect, useState, useSyncExternalStore } from 'react';
import { ChevronDown, KeyRound } from 'lucide-react';
import { getActiveTabId, getCurrentSessionId, setActiveTabId, subscribe } from './lib/app-state';
import sessionManager from './lib/session-manager';
import { Tabs, TabsContent, TabsList, TabsTrigger } from './components/ui/tabs';
import { Input } from './components/ui/input';
import { TooltipProvider } from './components/ui/tooltip';
import { Sidebar } from './layout/Sidebar';
import { Terminal } from './layout/Terminal';
import { FileManager } from './features/files/FileManager';
import { ConnectionDialog } from './layout/ConnectionDialog';
import { SettingsDialog } from './layout/SettingsDialog';
import { useConnections } from './features/connections/useConnections';

const subscribeActiveTabId = (callback: () => void) => subscribe('activeTabId', callback);

type UiBridge = { updateConnectionStatus: (connected: boolean, name?: string) => void; updateServerInfo: (connected: boolean, info?: { name?: string; host?: string }) => void; setFillPasswordPromptVisible: (visible: boolean) => void; fillPasswordPrimary: () => Promise<void> };
const subscribeCurrentSessionId = (callback: () => void) => subscribe('currentSessionId', callback);

type AppWindow = Window & { uiManager?: UiBridge };

export function App() {
  const activeTabId = useSyncExternalStore(subscribeActiveTabId, getActiveTabId, () => 'terminal');
  const connections = useConnections();
  const [status, setStatus] = useState({ connected: false, name: '', host: '' });
  const [passwordPrompt, setPasswordPrompt] = useState(false);
  const [fillMenuOpen, setFillMenuOpen] = useState(false);
  const [feedback, setFeedback] = useState(false);
  const [credentials, setCredentials] = useState<{ hasPassword?: boolean; accounts?: string[] }>({});
  const [newAccount, setNewAccount] = useState('');
  const [newPassword, setNewPassword] = useState('');
  const currentSessionId = useSyncExternalStore(subscribeCurrentSessionId, getCurrentSessionId, () => null);
  const currentConnectionId = currentSessionId ? sessionManager.getSession(currentSessionId)?.connectionId || null : null;

  const doFillPassword = async (kind: string) => {
    if (!currentSessionId) return;
    const result = await window.api.ssh.fillPassword(currentSessionId, kind);
    if (!result?.success) { alert(`填充密码失败: ${result?.error || '未知错误'}`); return; }
    setFeedback(true);
    setTimeout(() => setFeedback(false), 1500);
    try { (window as Window & { terminalManager?: { activeTerminal?: { focus: () => void } } }).terminalManager?.activeTerminal?.focus(); } catch {}
  };
  const openFillPasswordMenu = async () => {
    if (!currentConnectionId) return;
    const result = await window.api.cred.list(currentConnectionId);
    setCredentials(result || {});
    setFillMenuOpen(true);
  };
  const fillPasswordPrimary = async () => {
    if (!currentConnectionId) return;
    const result = await window.api.cred.list(currentConnectionId);
    const accounts = result?.accounts || [];
    const hasPassword = Boolean(result?.hasPassword);
    setCredentials(result || {});
    if ((hasPassword ? 1 : 0) + accounts.length <= 1) {
      if (hasPassword) return doFillPassword('password');
      if (accounts.length === 1) return doFillPassword(`acct:${accounts[0]}`);
    }
    setFillMenuOpen(true);
  };
  const saveCredential = async () => {
    if (!currentConnectionId || !newAccount.trim() || !newPassword) return;
    await window.api.cred.set(currentConnectionId, newAccount.trim(), newPassword);
    setNewAccount(''); setNewPassword(''); await openFillPasswordMenu();
  };

  useEffect(() => {
    const bridge: UiBridge = {
      updateConnectionStatus: (connected, name = '') => setStatus(current => ({ ...current, connected, name: connected ? name : '' })),
      updateServerInfo: (connected, info = {}) => setStatus(current => ({ ...current, connected, name: connected ? info.name || current.name : '', host: connected ? info.host || '' : '' })),
      setFillPasswordPromptVisible: setPasswordPrompt,
      fillPasswordPrimary,
    };
    (window as AppWindow).uiManager = bridge;
    return () => { if ((window as AppWindow).uiManager === bridge) delete (window as AppWindow).uiManager; };
  }, [currentConnectionId, currentSessionId]);

  return (
    <TooltipProvider delayDuration={300}>
    <div className="flex h-screen min-h-0 flex-col bg-background text-foreground">
      <div className="flex min-h-0 flex-1">
        <Sidebar manager={connections} />
        <main className="flex min-h-0 min-w-0 flex-1 flex-col">
          <Tabs value={activeTabId} onValueChange={setActiveTabId} className="flex min-h-0 flex-1 flex-col">
            <div className="flex h-12 shrink-0 items-center justify-between border-b border-border px-3">
              <TabsList className="h-9 bg-transparent p-0"><TabsTrigger value="terminal" data-tab="terminal" className="h-9 rounded-none border-b-2 border-transparent data-[state=active]:border-primary">终端</TabsTrigger><TabsTrigger value="file-manager" data-tab="file-manager" className="h-9 rounded-none border-b-2 border-transparent data-[state=active]:border-primary">文件管理</TabsTrigger></TabsList>
              <div className="relative flex items-center gap-2 text-xs text-muted-foreground" id="main-server-info"><span className={`h-2 w-2 rounded-full ${status.connected ? 'bg-success' : 'bg-muted-foreground/40'}`} /><span>{status.connected ? `${status.name} (${status.host})` : '未连接'}</span>{(passwordPrompt || feedback) && <div className="flex items-center"><button id="fill-password-btn" type="button" className="flex items-center gap-1 rounded-l border border-primary bg-primary px-2 py-1 text-primary-foreground" title="填充密码" onClick={() => void fillPasswordPrimary()}><KeyRound className="h-3.5 w-3.5" />{feedback ? '填充发送完毕' : '填充密码'}</button><button type="button" className="rounded-r border-y border-r border-primary bg-primary px-1 py-1 text-primary-foreground" aria-label="选择账号密码" onClick={() => void openFillPasswordMenu()}><ChevronDown className="h-3.5 w-3.5" /></button></div>}{fillMenuOpen && <div className="absolute right-0 top-8 z-30 grid w-64 gap-2 rounded-md border border-border bg-popover p-2 text-popover-foreground shadow-lg"><div className="grid gap-1">{credentials.hasPassword && <button type="button" className="rounded px-2 py-1 text-left text-xs hover:bg-muted" onClick={() => { setFillMenuOpen(false); void doFillPassword('password'); }}>连接密码</button>}{(credentials.accounts || []).map(account => <button key={account} type="button" className="rounded px-2 py-1 text-left text-xs hover:bg-muted" onClick={() => { setFillMenuOpen(false); void doFillPassword(`acct:${account}`); }}>{account}</button>)}</div><div className="grid gap-2 border-t border-border pt-2"><div className="text-xs font-medium">新增账号密码</div><Input className="h-8 text-xs" placeholder="账号" value={newAccount} onChange={event => setNewAccount(event.target.value)} /><Input className="h-8 text-xs" type="password" placeholder="密码" value={newPassword} onChange={event => setNewPassword(event.target.value)} /><button type="button" className="rounded bg-primary px-2 py-1 text-xs text-primary-foreground" onClick={() => void saveCredential()}>保存</button></div></div>}</div>
            </div>
            <div className="min-h-0 flex-1">
              <TabsContent value="terminal" id="terminal-tab" forceMount className="m-0 h-full data-[state=inactive]:hidden"><Terminal /></TabsContent>
              <TabsContent value="file-manager" id="file-manager-tab" forceMount className="m-0 h-full data-[state=inactive]:hidden"><FileManager /></TabsContent>
            </div>
          </Tabs>
        </main>
      </div>
      <div className="flex h-7 shrink-0 items-center gap-2 border-t border-border px-3 text-xs text-muted-foreground"><span className={`h-2 w-2 rounded-full ${status.connected ? 'bg-success' : 'bg-muted-foreground/40'}`} /><span>{status.connected ? `已连接: ${status.name}` : '未连接'}</span></div>
      {connections.loadingText && <div className="fixed inset-0 z-50 grid place-items-center bg-background/70 backdrop-blur-sm"><div className="flex items-center gap-3 rounded-lg border border-border bg-card px-5 py-4 shadow-xl"><span className="h-4 w-4 animate-spin rounded-full border-2 border-primary border-t-transparent" /><span>{connections.loadingText}</span><button type="button" className="rounded border border-border px-2 py-1 text-xs hover:bg-muted" onClick={() => void connections.cancel()}>取消</button></div></div>}
      <ConnectionDialog manager={connections} />
      <SettingsDialog />
    </div>
    </TooltipProvider>
  );
}
