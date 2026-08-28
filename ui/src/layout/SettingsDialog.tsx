import { Check, Folder, Plus, Type, X } from 'lucide-react';
import { useSyncExternalStore } from 'react';
import { getSettingsDialogOpen, setSettingsDialogOpen, subscribe } from '../../../assets/js/app-state.mjs';
import { Button } from '../components/ui/button';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '../components/ui/card';
import { Dialog, DialogClose, DialogContent, DialogFooter, DialogHeader, DialogTitle } from '../components/ui/dialog';
import { Label } from '../components/ui/label';
import { Tabs, TabsContent, TabsList, TabsTrigger } from '../components/ui/tabs';

// useSyncExternalStore 要求 subscribe 引用稳定：内联箭头函数会让 React 每次渲染都
// 重新订阅，通知会在 unsubscribe/subscribe 的间隙丢失。丢一次之后 store 已是新值，
// setter 的「值未变化直接 return」会让后续调用全部短路，UI 再也不更新。
const subscribeSettingsDialogOpen = (callback: () => void) => subscribe('settingsDialogOpen', callback);

const inputClassName = 'h-10 w-full rounded-md border border-input bg-background px-3 py-2 text-sm text-foreground shadow-sm outline-none transition-colors placeholder:text-muted-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2';

export function SettingsDialog() {
  const open = useSyncExternalStore(
    subscribeSettingsDialogOpen,
    getSettingsDialogOpen,
    () => false,
  );

  return (
    <Dialog open={open} onOpenChange={(nextOpen) => {
      if (!nextOpen) document.dispatchEvent(new CustomEvent('settings:close-request'));
      setSettingsDialogOpen(nextOpen);
    }}>
      <DialogContent
        id="settings-dialog"
        forceMount
        showCloseButton={false}
        className={`max-w-3xl p-0 ${open ? 'active' : ''}`}
      >
        <DialogHeader className="flex-row items-center justify-between border-b border-border px-6 py-4">
          <DialogTitle className="text-xl">设置</DialogTitle>
          <DialogClose asChild>
            <Button type="button" id="settings-close" variant="ghost" size="icon" title="关闭">
              <X className="h-4 w-4" />
              <span className="sr-only">关闭</span>
            </Button>
          </DialogClose>
        </DialogHeader>

        <Tabs defaultValue="font" className="flex min-h-[26rem] flex-col sm:flex-row" orientation="vertical">
          <TabsList className="h-auto w-full shrink-0 justify-start gap-1 rounded-none border-b border-border bg-muted/40 p-3 sm:w-40 sm:flex-col sm:items-stretch sm:border-b-0 sm:border-r sm:rounded-none">
            <TabsTrigger value="font" data-settings-tab="font" className="justify-start gap-2" role="tab">
              <Type className="h-4 w-4" />字体
            </TabsTrigger>
            <TabsTrigger value="groups" data-settings-tab="groups" className="justify-start gap-2" role="tab">
              <Folder className="h-4 w-4" />分组管理
            </TabsTrigger>
          </TabsList>

          <div className="min-w-0 flex-1 p-6">
            <TabsContent value="font" id="settings-panel-font" data-settings-panel="font" forceMount className="mt-0 data-[state=inactive]:hidden">
              <Card className="border-border/80 shadow-sm">
                <CardHeader>
                  <CardTitle className="text-base">字体</CardTitle>
                  <CardDescription>调整终端字号和字体，预览会实时更新。</CardDescription>
                </CardHeader>
                <form id="settings-form">
                  <CardContent className="grid gap-6">
                    <div className="grid gap-4 sm:grid-cols-[7rem_1fr]">
                      <div className="grid gap-2">
                        <Label htmlFor="settings-font-size">字号</Label>
                        <div className="flex h-10 items-center overflow-hidden rounded-md border border-input bg-background">
                          <Button type="button" variant="ghost" size="icon" className="font-size-step h-full w-8 rounded-none" data-step="-1" aria-label="减小">−</Button>
                          <input type="number" id="settings-font-size" className="h-full min-w-0 flex-1 border-0 bg-transparent px-1 text-center text-sm text-foreground outline-none focus:ring-0" min="8" max="40" step="1" required autoComplete="off" />
                          <Button type="button" variant="ghost" size="icon" className="font-size-step h-full w-8 rounded-none" data-step="1" aria-label="增大">+</Button>
                        </div>
                      </div>
                      <div className="grid gap-2">
                        <Label htmlFor="settings-font-family">字体</Label>
                        <select id="settings-font-family" className={`${inputClassName} cursor-pointer`} />
                      </div>
                    </div>
                    <div className="grid gap-2 hidden" id="settings-custom-wrap">
                      <Label htmlFor="settings-font-family-custom">自定义 fontFamily</Label>
                      <input type="text" id="settings-font-family-custom" className={inputClassName} placeholder='"Cascadia Code", monospace' autoComplete="off" autoCapitalize="off" autoCorrect="off" spellCheck={false} />
                    </div>
                    <div className="rounded-lg bg-slate-950 p-4 font-mono text-sm leading-6 text-slate-100 shadow-inner" id="settings-preview">
                      <div className="truncate">user@host:~$ ls -la /etc</div>
                      <div className="truncate text-slate-400">drwxr-xr-x  123 root  4096 May 24 10:42 .</div>
                      <div className="truncate">The quick brown fox 0123456789 → ✓</div>
                      <div className="truncate">Emoji: 😀 😃 🚀 🐱 ❤️ 🌈 🍎 ✅ ⚡ 中文</div>
                      <div className="truncate">Nerd Font 图标: &#xe0b0; &#xf07b; &#xf015; &#xf120; &#xe0a0; &#xf09b; &#xf135;</div>
                    </div>
                  </CardContent>
                  <DialogFooter className="border-t border-border px-6 py-4">
                    <Button type="button" id="settings-cancel" variant="outline">取消</Button>
                    <Button type="submit" id="settings-save"><Check className="h-4 w-4" />保存</Button>
                  </DialogFooter>
                </form>
              </Card>
            </TabsContent>

            <TabsContent value="groups" id="settings-panel-groups" data-settings-panel="groups" forceMount className="mt-0 data-[state=inactive]:hidden">
              <Card className="border-border/80 shadow-sm">
                <CardHeader>
                  <CardTitle className="text-base">分组管理</CardTitle>
                  <CardDescription>整理机器分组，拖拽连接列表即可调整顺序。</CardDescription>
                </CardHeader>
                <form id="group-manager-form">
                  <CardContent className="grid gap-6">
                    <div className="flex gap-2">
                      <input type="text" id="group-new-name" className={inputClassName} placeholder="新分组名称" autoComplete="off" maxLength={40} />
                      <Button type="button" id="group-add-btn" className="shrink-0"><Plus className="h-4 w-4" />添加分组</Button>
                    </div>
                    <div className="grid gap-2">
                      <div className="text-sm font-medium text-foreground">已有分组</div>
                      <div id="group-manager-groups" />
                    </div>
                    <div className="grid gap-2">
                      <div className="text-sm font-medium text-foreground">快速分配机器</div>
                      <div id="group-manager-connections" />
                    </div>
                  </CardContent>
                  <DialogFooter className="border-t border-border px-6 py-4">
                    <Button type="submit" id="group-manager-save"><Check className="h-4 w-4" />保存</Button>
                  </DialogFooter>
                </form>
              </Card>
            </TabsContent>
          </div>
        </Tabs>
      </DialogContent>
    </Dialog>
  );
}
