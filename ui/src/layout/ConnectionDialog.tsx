import { ArrowRight, FolderOpen, KeyRound, Save, X } from 'lucide-react';
import { useEffect, useState, useSyncExternalStore } from 'react';
import {
  getConnectionDialogOpen,
  getEditingConnection,
  setConnectionDialogOpen,
  setEditingConnection,
  subscribe,
} from '../../../assets/js/app-state.mjs';
import { Button } from '../components/ui/button';
import { Checkbox } from '../components/ui/checkbox';
import { Dialog, DialogContent, DialogFooter, DialogHeader, DialogTitle } from '../components/ui/dialog';
import { Input } from '../components/ui/input';
import { Label } from '../components/ui/label';
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '../components/ui/select';

// useSyncExternalStore 要求 subscribe 引用稳定：内联箭头函数会让 React 每次渲染都
// 重新订阅，通知会在 unsubscribe/subscribe 的间隙丢失。丢一次之后 store 已是新值，
// setter 的「值未变化直接 return」会让后续调用全部短路，UI 再也不更新。
const subscribeConnectionDialogOpen = (callback: () => void) => subscribe('connectionDialogOpen', callback);
const subscribeEditingConnection = (callback: () => void) => subscribe('editingConnection', callback);

type AuthType = 'password' | 'privateKey';

type ConnectionForm = {
  name: string;
  group: string;
  host: string;
  port: string;
  username: string;
  authType: AuthType;
  password: string;
  privateKey: string;
  passphrase: string;
  savePassword: boolean;
};

type ConnectionRecord = {
  id: string;
  name?: string;
  group?: string;
  host?: string;
  port?: number;
  username?: string;
  authType?: AuthType;
  password?: string;
  privateKey?: string;
  passphrase?: string;
};

type ConnectionDetails = ConnectionForm;

type ConnectionManager = {
  submitConnection: (details: ConnectionDetails) => Promise<void>;
  saveEditedConnection: (id: string, details: ConnectionDetails) => Promise<void>;
};

type BrowserApi = {
  file?: {
    getHomeDir?: () => Promise<string>;
  };
  dialog?: {
    selectFile?: () => Promise<{ canceled: boolean; filePaths: string[] }>;
  };
};

const createEmptyForm = (): ConnectionForm => ({
  name: '',
  group: '',
  host: '',
  port: '22',
  username: '',
  authType: 'password',
  password: '',
  privateKey: '',
  passphrase: '',
  savePassword: true,
});

const formFromConnection = (connection: ConnectionRecord): ConnectionForm => {
  const authType = connection.authType || (connection.privateKey ? 'privateKey' : 'password');
  return {
    name: connection.name || '',
    group: connection.group || '',
    host: connection.host || '',
    port: String(connection.port ?? 22),
    username: connection.username || '',
    authType,
    password: authType === 'password' ? connection.password || '' : '',
    privateKey: authType === 'privateKey' ? connection.privateKey || '' : '',
    passphrase: authType === 'privateKey' ? connection.passphrase || '' : '',
    savePassword: Boolean(connection.password || connection.passphrase),
  };
};

export function ConnectionDialog() {
  const open = useSyncExternalStore(
    subscribeConnectionDialogOpen,
    getConnectionDialogOpen,
    () => false,
  );
  const editingConnection = useSyncExternalStore(
    subscribeEditingConnection,
    getEditingConnection,
    () => null,
  ) as ConnectionRecord | null;
  const [form, setForm] = useState<ConnectionForm>(createEmptyForm);

  useEffect(() => {
    setForm(open && editingConnection ? formFromConnection(editingConnection) : createEmptyForm());
  }, [open, editingConnection]);

  const updateField = <K extends keyof ConnectionForm>(field: K, value: ConnectionForm[K]) => {
    setForm(previous => ({ ...previous, [field]: value }));
  };

  const handleOpenChange = (nextOpen: boolean) => {
    setConnectionDialogOpen(nextOpen);
    if (!nextOpen) setEditingConnection(null);
  };

  const handleAuthTypeChange = (authType: AuthType) => {
    setForm(previous => ({
      ...previous,
      authType,
      password: authType === 'password' ? previous.password : '',
      privateKey: authType === 'privateKey' ? previous.privateKey : '',
      passphrase: authType === 'privateKey' ? previous.passphrase : '',
    }));

    if (authType !== 'privateKey' || form.privateKey) return;

    const api = (window as Window & { api?: BrowserApi }).api;
    if (!api?.file?.getHomeDir) return;
    api.file.getHomeDir()
      .then(homeDir => {
        const privateKey = homeDir.includes('\\')
          ? `${homeDir}\\.ssh\\id_rsa`
          : `${homeDir}/.ssh/id_rsa`;
        setForm(previous => previous.authType === 'privateKey' && !previous.privateKey
          ? { ...previous, privateKey }
          : previous);
      })
      .catch(error => console.error('获取用户主目录失败:', error));
  };

  const handleBrowsePrivateKey = async () => {
    const api = (window as Window & { api?: BrowserApi }).api;
    const result = await api?.dialog?.selectFile?.();
    if (!result?.canceled && result?.filePaths[0]) {
      updateField('privateKey', result.filePaths[0]);
    }
  };

  const handleSubmit = async (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const manager = (window as Window & { connectionManager?: ConnectionManager }).connectionManager;
    if (!manager) return;

    if (editingConnection) {
      await manager.saveEditedConnection(editingConnection.id, form);
    } else {
      await manager.submitConnection(form);
    }
  };

  return (
    <>
      <Dialog open={open} onOpenChange={handleOpenChange}>
        <DialogContent
          id="connection-dialog"
          forceMount
          showCloseButton
          className={`max-w-2xl ${open ? 'active' : ''}`}
        >
          <DialogHeader className="border-b border-border pb-4">
            <DialogTitle className="flex items-center gap-2 text-xl">
              <KeyRound className="h-5 w-5 text-primary" />
              {editingConnection ? '编辑 SSH 连接' : '新建 SSH 连接'}
            </DialogTitle>
          </DialogHeader>

          <form id="connection-form" className="!p-0 grid gap-5" onSubmit={handleSubmit}>
            <div className="grid gap-2">
              <Label htmlFor="conn-name">名称</Label>
              <Input id="conn-name" value={form.name} onChange={event => updateField('name', event.target.value)} placeholder="我的服务器" required autoComplete="off" autoCapitalize="off" autoCorrect="off" spellCheck={false} />
            </div>
            <div className="grid gap-2">
              <Label htmlFor="conn-group">分组 <span className="text-xs font-normal text-muted-foreground">（可选）</span></Label>
              <Input id="conn-group" value={form.group} onChange={event => updateField('group', event.target.value)} placeholder="例如：生产环境、开发环境" autoComplete="off" autoCapitalize="off" autoCorrect="off" spellCheck={false} />
            </div>
            <div className="grid gap-4 sm:grid-cols-[1fr_8rem]">
              <div className="grid gap-2">
                <Label htmlFor="conn-host">主机</Label>
                <Input id="conn-host" value={form.host} onChange={event => updateField('host', event.target.value)} placeholder="example.com 或 192.168.1.1" required autoComplete="off" autoCapitalize="off" autoCorrect="off" spellCheck={false} />
              </div>
              <div className="grid gap-2">
                <Label htmlFor="conn-port">端口</Label>
                <Input id="conn-port" type="number" value={form.port} onChange={event => updateField('port', event.target.value)} min="1" max="65535" autoComplete="off" />
              </div>
            </div>
            <div className="grid gap-2">
              <Label htmlFor="conn-username">用户名</Label>
              <Input id="conn-username" value={form.username} onChange={event => updateField('username', event.target.value)} placeholder="root" required autoComplete="off" autoCapitalize="off" autoCorrect="off" spellCheck={false} />
            </div>
            <div className="grid gap-2">
              <Label htmlFor="auth-type">认证方式</Label>
              <Select value={form.authType} onValueChange={value => handleAuthTypeChange(value as AuthType)}>
                <SelectTrigger id="auth-type">
                  <SelectValue placeholder="选择认证方式" />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="password">密码</SelectItem>
                  <SelectItem value="privateKey">私钥</SelectItem>
                </SelectContent>
              </Select>
            </div>
            {form.authType === 'password' ? (
              <div className="grid gap-2">
                <Label htmlFor="conn-password">密码</Label>
                <Input id="conn-password" type="password" value={form.password} onChange={event => updateField('password', event.target.value)} autoComplete="new-password" autoCapitalize="off" autoCorrect="off" spellCheck={false} />
              </div>
            ) : (
              <>
                <div className="grid gap-2">
                  <Label htmlFor="conn-private-key-path">私钥文件</Label>
                  <div className="flex gap-2">
                    <Input id="conn-private-key-path" value={form.privateKey} onChange={event => updateField('privateKey', event.target.value)} readOnly autoComplete="off" autoCapitalize="off" autoCorrect="off" spellCheck={false} />
                    <Button type="button" id="browse-private-key" variant="outline" className="shrink-0" onClick={handleBrowsePrivateKey}><FolderOpen className="h-4 w-4" />浏览</Button>
                  </div>
                </div>
                <div className="grid gap-2">
                  <Label htmlFor="conn-passphrase">私钥密码 (可选)</Label>
                  <Input id="conn-passphrase" type="password" value={form.passphrase} onChange={event => updateField('passphrase', event.target.value)} autoComplete="new-password" autoCapitalize="off" autoCorrect="off" spellCheck={false} />
                </div>
              </>
            )}
            <div className="flex items-center gap-2">
              <Checkbox id="conn-save-password" checked={form.savePassword} onCheckedChange={checked => updateField('savePassword', checked === true)} />
              <Label htmlFor="conn-save-password" className="font-normal">保存密码</Label>
            </div>
            <DialogFooter className="border-t border-border pt-5">
              <Button type="button" id="cancel-connection" variant="outline" onClick={() => handleOpenChange(false)}><X className="h-4 w-4" />取消</Button>
              <Button type="submit" id="connection-submit-btn">{editingConnection ? <Save className="h-4 w-4" /> : <ArrowRight className="h-4 w-4" />}{editingConnection ? '保存' : '连接'}</Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>

      <div id="loading-overlay" className="loading-overlay hidden">
        <div className="spinner" />
        <div className="loading-text">连接中...</div>
      </div>
    </>
  );
}
