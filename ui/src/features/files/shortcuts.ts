import { useEffect, useRef } from 'react';

type ShortcutKey = Pick<KeyboardEvent, 'key' | 'metaKey' | 'ctrlKey' | 'shiftKey' | 'altKey'>;
type ShortcutEvent = ShortcutKey & Pick<KeyboardEvent, 'target' | 'preventDefault'>;

// 终端（xterm）的输入也是 textarea，一并放行。
const EDITABLE = 'input, textarea, select, [contenteditable=""], [contenteditable="true"]';
// 对话框、菜单打开时，Cmd+A 不能去选它们背后的文件。
const OVERLAY = '[role="dialog"], [role="alertdialog"], [role="menu"]';

// 用 key 而不是 code：AZERTY 等布局上 A 键的物理位置不同。
export function isSelectAllShortcut(event: ShortcutKey) {
  return (event.metaKey || event.ctrlKey) && !event.shiftKey && !event.altKey && event.key.toLowerCase() === 'a';
}

function isEditableTarget(target: EventTarget | null) {
  return Boolean((target as Element | null)?.closest?.(EDITABLE));
}

/** 应用级（main.tsx 注册一次）：焦点不在可编辑元素上时拦下浏览器默认全选，界面文字不被整片选中。 */
export function preventUiSelectAll(event: ShortcutEvent) {
  if (isSelectAllShortcut(event) && !isEditableTarget(event.target)) event.preventDefault();
}

/**
 * 文件管理页可见、焦点不在文件列表也不在可编辑元素上时（点过工具栏、空白处），Cmd+A 改为全选文件。
 * 焦点在文件列表上时由 FilePane 自己处理，这里跳过。
 */
export function useSelectAllFallback(enabled: boolean, onSelectAll: () => void) {
  const callback = useRef(onSelectAll);
  callback.current = onSelectAll;
  useEffect(() => {
    if (!enabled) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (!isSelectAllShortcut(event) || isEditableTarget(event.target)) return;
      if ((event.target as Element | null)?.closest?.('[data-file-list]')) return;
      if (document.querySelector(OVERLAY)) return;
      callback.current();
    };
    document.addEventListener('keydown', onKeyDown);
    return () => document.removeEventListener('keydown', onKeyDown);
  }, [enabled]);
}
