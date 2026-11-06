// 切回终端时焦点要直接回到终端里，否则（比如在文件管理上传完切回来）直接打字没反应。
// 两种时机：切到终端标签；窗口重新获得焦点（从别的应用切回来）。有对话框 / 菜单开着、
// 或用户正停在某个输入框里时不抢。

type FocusTarget = { tagName?: string; isContentEditable?: boolean } | null;

export function shouldRefocusTerminal({ reason, activeTab, active, dialogOpen }: {
  reason: 'tab' | 'window';
  activeTab: string;
  active: FocusTarget;
  dialogOpen: boolean;
}) {
  if (activeTab !== 'terminal' || dialogOpen) return false;
  if (reason === 'tab') return true;
  const tag = active?.tagName;
  return !(tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT' || active?.isContentEditable);
}

/** 当前是否有对话框或菜单开着（Radix 的对话框 / 右键菜单都带这些 role）。 */
export function overlayOpen(doc: Document) {
  return Boolean(doc.querySelector('[role="dialog"], [role="alertdialog"], [role="menu"]'));
}
