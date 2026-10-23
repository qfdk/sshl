import { useRef, useSyncExternalStore } from 'react';
import { Button } from '../components/ui/button';
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '../components/ui/dialog';
import { currentDialog, restoreFocusAfterDialogs, settleDialog, subscribeDialogs } from '../lib/dialogs';

// 渲染 lib/dialogs 排队的提示/确认框，样式与新建连接、设置对话框一致。
export function DialogHost() {
  const request = useSyncExternalStore(subscribeDialogs, currentDialog, () => null);
  const confirmRef = useRef<HTMLButtonElement>(null);

  return (
    <Dialog open={Boolean(request)} onOpenChange={open => { if (!open) settleDialog(false); }}>
      {request && (
        <DialogContent
          key={request.id}
          showCloseButton={false}
          className="max-w-[420px] gap-4 p-5"
          // 没有正文时显式置空，免得 Radix 警告缺少 Description
          {...(request.message ? {} : { 'aria-describedby': undefined })}
          // 回车 = 确认（与系统弹窗的默认按钮一致），Esc / 点遮罩 = 取消
          onOpenAutoFocus={event => { event.preventDefault(); confirmRef.current?.focus(); }}
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
          <DialogFooter className="-mx-5 -mb-5 mt-1 rounded-b-2xl border-t border-border bg-muted px-5 py-3">
            {request.kind === 'confirm' && (
              <Button type="button" variant="outline" className="h-9" onClick={() => settleDialog(false)}>{request.cancelLabel}</Button>
            )}
            <Button ref={confirmRef} type="button" className="h-9" onClick={() => settleDialog(true)}>{request.confirmLabel}</Button>
          </DialogFooter>
        </DialogContent>
      )}
    </Dialog>
  );
}
