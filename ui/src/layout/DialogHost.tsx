import { useRef, useState, useSyncExternalStore } from 'react';
import { KeyRound } from 'lucide-react';
import { Button } from '../components/ui/button';
import { Input } from '../components/ui/input';
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '../components/ui/dialog';
import { currentDialog, restoreFocusAfterDialogs, settleDialog, subscribeDialogs } from '../lib/dialogs';

// 渲染 lib/dialogs 排队的提示/确认/输入框，样式与新建连接、设置对话框一致。
export function DialogHost() {
  const request = useSyncExternalStore(subscribeDialogs, currentDialog, () => null);
  const confirmRef = useRef<HTMLButtonElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const [value, setValue] = useState('');
  const isPrompt = request?.kind === 'prompt';
  const settle = (ok: boolean, choice?: string) => { const v = value; setValue(''); settleDialog(ok, isPrompt ? v : undefined, choice); };

  return (
    <Dialog open={Boolean(request)} onOpenChange={open => { if (!open) settle(false); }}>
      {request && (
        <DialogContent
          key={request.id}
          showCloseButton={false}
          className="max-w-[420px] gap-4 p-5"
          // 没有正文时显式置空，免得 Radix 警告缺少 Description
          {...(request.message ? {} : { 'aria-describedby': undefined })}
          // 回车 = 确认（与系统弹窗的默认按钮一致），Esc / 点遮罩 = 取消；输入框聚焦输入框
          onOpenAutoFocus={event => { event.preventDefault(); (isPrompt ? inputRef.current : confirmRef.current)?.focus(); }}
          // Radix 关闭时会去聚焦（不存在的）DialogTrigger，焦点就丢了；改为还给弹出前的元素
          onCloseAutoFocus={event => { event.preventDefault(); restoreFocusAfterDialogs(); }}
        >
          {/* min-w-0：grid 子项默认按内容撑宽，否则长 detail 会把对话框撑开、truncate 失效 */}
          <DialogHeader className="min-w-0 gap-1.5">
            <DialogTitle className="text-base">{request.title}</DialogTitle>
            {/* 报错信息要能拖选复制（界面文字默认不可选） */}
            {request.detail && <p className="select-text truncate text-sm text-foreground/80" title={request.detail}>{request.detail}</p>}
            {request.message && <DialogDescription className="select-text whitespace-pre-line break-words">{request.message}</DialogDescription>}
          </DialogHeader>
          {/* 已保存的密码：点一下就以它确定（只回传标识，密码不经过界面） */}
          {isPrompt && request.choices?.length ? (
            <div className="grid gap-1.5">
              <div className="text-xs text-muted-foreground">已保存的密码</div>
              <div className="flex flex-wrap gap-1.5">
                {request.choices.map(choice => (
                  <Button key={choice.value} type="button" variant="outline" className="h-8 gap-1.5 px-2.5 text-xs" onClick={() => settle(true, choice.value)}>
                    <KeyRound className="h-3.5 w-3.5" />{choice.label}
                  </Button>
                ))}
              </div>
            </div>
          ) : null}
          {isPrompt && (
            <Input
              ref={inputRef}
              type={request.inputType || 'text'}
              placeholder={request.placeholder}
              autoComplete="off"
              className="h-9"
              value={value}
              onChange={event => setValue(event.target.value)}
              onKeyDown={event => { if (event.key === 'Enter' && !event.nativeEvent.isComposing) { event.preventDefault(); settle(true); } }}
            />
          )}
          <DialogFooter className="-mx-5 -mb-5 mt-1 rounded-b-2xl border-t border-border bg-muted px-5 py-3">
            {request.kind !== 'alert' && (
              <Button type="button" variant="outline" className="h-9" onClick={() => settle(false)}>{request.cancelLabel}</Button>
            )}
            <Button ref={confirmRef} type="button" className="h-9" onClick={() => settle(true)}>{request.confirmLabel}</Button>
          </DialogFooter>
        </DialogContent>
      )}
    </Dialog>
  );
}
