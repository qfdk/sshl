import { ArrowRight, FolderOpen, KeyRound, X } from 'lucide-react';
import { useSyncExternalStore } from 'react';
import { getConnectionDialogOpen, setConnectionDialogOpen, subscribe } from '../../../assets/js/app-state.mjs';
import { Button } from '../components/ui/button';
import { Dialog, DialogContent, DialogFooter, DialogHeader, DialogTitle } from '../components/ui/dialog';
import { Label } from '../components/ui/label';

// useSyncExternalStore 要求 subscribe 引用稳定：内联箭头函数会让 React 每次渲染都
// 重新订阅，通知会在 unsubscribe/subscribe 的间隙丢失。丢一次之后 store 已是新值，
// setter 的「值未变化直接 return」会让后续调用全部短路，UI 再也不更新。
const subscribeConnectionDialogOpen = (callback: () => void) => subscribe('connectionDialogOpen', callback);

const inputClassName = 'h-10 w-full rounded-md border border-input bg-background px-3 py-2 text-sm text-foreground shadow-sm outline-none transition-colors placeholder:text-muted-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2';

export function ConnectionDialog() {
  const open = useSyncExternalStore(
    subscribeConnectionDialogOpen,
    getConnectionDialogOpen,
    () => false,
  );

  return (
    <>
      <Dialog open={open} onOpenChange={setConnectionDialogOpen}>
        <DialogContent
          id="connection-dialog"
          forceMount
          showCloseButton
          className={`max-w-2xl ${open ? 'active' : ''}`}
        >
          <DialogHeader className="border-b border-border pb-4">
            <DialogTitle className="flex items-center gap-2 text-xl">
              <KeyRound className="h-5 w-5 text-primary" />
              新建 SSH 连接
            </DialogTitle>
          </DialogHeader>

          <form id="connection-form" className="!p-0 grid gap-5">
            <div className="grid gap-2">
              <Label htmlFor="conn-name">名称</Label>
              <input type="text" id="conn-name" className={inputClassName} placeholder="我的服务器" required autoComplete="off" autoCapitalize="off" autoCorrect="off" spellCheck={false} />
            </div>
            <div className="grid gap-2">
              <Label htmlFor="conn-group">分组 <span className="text-xs font-normal text-muted-foreground">（可选）</span></Label>
              <input type="text" id="conn-group" className={inputClassName} placeholder="例如：生产环境、开发环境" autoComplete="off" autoCapitalize="off" autoCorrect="off" spellCheck={false} />
            </div>
            <div className="grid gap-4 sm:grid-cols-[1fr_8rem]">
              <div className="grid gap-2">
                <Label htmlFor="conn-host">主机</Label>
                <input type="text" id="conn-host" className={inputClassName} placeholder="example.com 或 192.168.1.1" required autoComplete="off" autoCapitalize="off" autoCorrect="off" spellCheck={false} />
              </div>
              <div className="grid gap-2">
                <Label htmlFor="conn-port">端口</Label>
                <input type="number" id="conn-port" className={inputClassName} defaultValue="22" min="1" max="65535" autoComplete="off" />
              </div>
            </div>
            <div className="grid gap-2">
              <Label htmlFor="conn-username">用户名</Label>
              <input type="text" id="conn-username" className={inputClassName} placeholder="root" required autoComplete="off" autoCapitalize="off" autoCorrect="off" spellCheck={false} />
            </div>
            <div className="grid gap-2">
              <Label htmlFor="auth-type">认证方式</Label>
              <select id="auth-type" className={`${inputClassName} cursor-pointer`}>
                <option value="password">密码</option>
                <option value="privateKey">私钥</option>
              </select>
            </div>
            <div className="auth-password grid gap-2">
              <Label htmlFor="conn-password">密码</Label>
              <input type="password" id="conn-password" className={inputClassName} autoComplete="new-password" autoCapitalize="off" autoCorrect="off" spellCheck={false} />
            </div>
            <div className="auth-key hidden grid gap-2">
              <Label htmlFor="conn-private-key-path">私钥文件</Label>
              <div className="flex gap-2">
                <input type="text" id="conn-private-key-path" className={inputClassName} readOnly autoComplete="off" autoCapitalize="off" autoCorrect="off" spellCheck={false} />
                <Button type="button" id="browse-private-key" variant="outline" className="shrink-0"><FolderOpen className="h-4 w-4" />浏览</Button>
              </div>
            </div>
            <div className="auth-key hidden grid gap-2">
              <Label htmlFor="conn-passphrase">私钥密码 (可选)</Label>
              <input type="password" id="conn-passphrase" className={inputClassName} autoComplete="new-password" autoCapitalize="off" autoCorrect="off" spellCheck={false} />
            </div>
            <div className="flex items-center gap-2">
              <input type="checkbox" id="conn-save-password" className="h-4 w-4 rounded border-input accent-primary" defaultChecked />
              <Label htmlFor="conn-save-password" className="font-normal">保存密码</Label>
            </div>
            <DialogFooter className="border-t border-border pt-5">
              <Button type="button" id="cancel-connection" variant="outline"><X className="h-4 w-4" />取消</Button>
              <Button type="submit" id="connection-submit-btn"><ArrowRight className="h-4 w-4" />连接</Button>
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
