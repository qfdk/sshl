import { ArrowRight, FolderOpen, KeyRound, Lock, Save, X } from 'lucide-react';
import { useEffect, useState, useSyncExternalStore, type FormEvent } from 'react';
import {
  getConnectionDialogOpen,
  getEditingConnection,
  setConnectionDialogOpen,
  setEditingConnection,
  subscribe,
} from '../lib/app-state';
import type { ConnectionManager } from '../features/connections/useConnections';
import { Button } from '../components/ui/button';
import { Checkbox } from '../components/ui/checkbox';
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '../components/ui/dialog';
import { Input } from '../components/ui/input';
import { Label } from '../components/ui/label';
import { cn } from '../lib/utils';

const AUTH_OPTIONS = [
  { value: 'password', label: '密码', Icon: Lock },
  { value: 'privateKey', label: '私钥', Icon: KeyRound },
] as const;

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

export function ConnectionDialog({ manager }: { manager: ConnectionManager }) {
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

  const handleSubmit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
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
          showCloseButton
          className="max-w-[520px] gap-4 p-5"
        >
          <DialogHeader className="gap-1 pr-8">
            <DialogTitle className="text-base">{editingConnection ? '编辑连接' : '新建连接'}</DialogTitle>
            <DialogDescription>{editingConnection ? '修改服务器信息后点保存。' : '填写服务器信息，连接成功后自动保存到列表。'}</DialogDescription>
          </DialogHeader>

          {/* 空 form 只承载提交与校验（输入框用 form 属性关联）；contents 让它不占网格行、不多吃一份间距 */}
          <form id="connection-form" className="contents" onSubmit={handleSubmit} />
          <div className="grid gap-3">
            <div className="grid gap-3 sm:grid-cols-2">
              <div className="grid gap-1.5">
                <Label htmlFor="conn-name">名称</Label>
                <Input id="conn-name" form="connection-form" className="h-9" value={form.name} onChange={event => updateField('name', event.target.value)} placeholder="我的服务器" required autoComplete="off" autoCapitalize="off" autoCorrect="off" spellCheck={false} />
              </div>
              <div className="grid gap-1.5">
                <Label htmlFor="conn-group">分组 <span className="text-xs font-normal text-muted-foreground">（可选）</span></Label>
                <Input id="conn-group" form="connection-form" className="h-9" value={form.group} onChange={event => updateField('group', event.target.value)} placeholder="例如：生产环境" autoComplete="off" autoCapitalize="off" autoCorrect="off" spellCheck={false} />
              </div>
            </div>
            <div className="grid gap-3 sm:grid-cols-[1fr_6rem]">
              <div className="grid gap-1.5">
                <Label htmlFor="conn-host">主机</Label>
                <Input id="conn-host" form="connection-form" className="h-9" value={form.host} onChange={event => updateField('host', event.target.value)} placeholder="example.com 或 192.168.1.1" required autoComplete="off" autoCapitalize="off" autoCorrect="off" spellCheck={false} />
              </div>
              <div className="grid gap-1.5">
                <Label htmlFor="conn-port">端口</Label>
                <Input id="conn-port" form="connection-form" type="number" className="h-9 [appearance:textfield] [&::-webkit-inner-spin-button]:appearance-none [&::-webkit-outer-spin-button]:appearance-none" value={form.port} onChange={event => updateField('port', event.target.value)} min="1" max="65535" autoComplete="off" />
              </div>
            </div>
            <div className="grid gap-1.5">
              <Label htmlFor="conn-username">用户名</Label>
              <Input id="conn-username" form="connection-form" className="h-9" value={form.username} onChange={event => updateField('username', event.target.value)} placeholder="root" required autoComplete="off" autoCapitalize="off" autoCorrect="off" spellCheck={false} />
            </div>
            <div className="grid gap-1.5">
              <Label id="auth-type-label">认证方式</Label>
              <div id="auth-type" role="radiogroup" aria-labelledby="auth-type-label" className="grid grid-cols-2 gap-0.5 rounded-[10px] bg-secondary p-[3px]">
                {AUTH_OPTIONS.map(({ value, label, Icon }) => (
                  <button
                    key={value}
                    type="button"
                    role="radio"
                    aria-checked={form.authType === value}
                    className={cn('flex h-8 items-center justify-center gap-1.5 rounded-[7px] text-sm transition-colors', form.authType === value ? 'bg-card font-medium shadow-sm' : 'text-muted-foreground hover:text-foreground')}
                    onClick={() => { if (form.authType !== value) handleAuthTypeChange(value); }}
                  >
                    <Icon className="h-3.5 w-3.5" />{label}
                  </button>
                ))}
              </div>
            </div>
            {form.authType === 'password' ? (
              <div className="grid gap-1.5">
                <Label htmlFor="conn-password">密码</Label>
                <Input id="conn-password" form="connection-form" type="password" className="h-9" value={form.password} onChange={event => updateField('password', event.target.value)} autoComplete="new-password" autoCapitalize="off" autoCorrect="off" spellCheck={false} />
              </div>
            ) : (
              <>
                <div className="grid gap-1.5">
                  <Label htmlFor="conn-private-key-path">私钥文件</Label>
                  <div className="flex gap-2">
                    <Input id="conn-private-key-path" form="connection-form" className="h-9" value={form.privateKey} onChange={event => updateField('privateKey', event.target.value)} readOnly autoComplete="off" autoCapitalize="off" autoCorrect="off" spellCheck={false} />
                    <Button type="button" id="browse-private-key" variant="outline" className="h-9 shrink-0" onClick={handleBrowsePrivateKey}><FolderOpen className="h-4 w-4" />浏览</Button>
                  </div>
                </div>
                <div className="grid gap-1.5">
                  <Label htmlFor="conn-passphrase">私钥密码 <span className="text-xs font-normal text-muted-foreground">（可选）</span></Label>
                  <Input id="conn-passphrase" form="connection-form" type="password" className="h-9" value={form.passphrase} onChange={event => updateField('passphrase', event.target.value)} autoComplete="new-password" autoCapitalize="off" autoCorrect="off" spellCheck={false} />
                </div>
              </>
            )}
            <div className="flex items-center gap-2">
              <Checkbox id="conn-save-password" checked={form.savePassword} onCheckedChange={checked => updateField('savePassword', checked === true)} />
              <Label htmlFor="conn-save-password" className="font-normal">保存密码</Label>
              <span className="text-sm text-muted-foreground">· AES 加密保存在本机</span>
            </div>
            <DialogFooter className="-mx-5 -mb-5 mt-1 rounded-b-2xl border-t border-border bg-muted px-5 py-3">
              <Button type="button" id="cancel-connection" variant="outline" className="h-9" onClick={() => handleOpenChange(false)}><X className="h-4 w-4" />取消</Button>
              <Button type="submit" form="connection-form" id="connection-submit-btn" className="h-9">{editingConnection ? <Save className="h-4 w-4" /> : <ArrowRight className="h-4 w-4" />}{editingConnection ? '保存' : '连接'}</Button>
            </DialogFooter>
          </div>
        </DialogContent>
      </Dialog>

    </>
  );
}
