import { useEffect, useRef } from 'react';

type ShortcutKey = Pick<KeyboardEvent, 'key' | 'metaKey' | 'ctrlKey' | 'shiftKey' | 'altKey'>;
type ShortcutEvent = ShortcutKey & Pick<KeyboardEvent, 'target' | 'preventDefault'>;

// 终端（xterm）的输入也是 textarea，一并放行。
const EDITABLE = 'input, textarea, select, [contenteditable=""], [contenteditable="true"]';
// 对话框、菜单打开时，Cmd+A / Esc 不能去动它们背后的文件选择。
const OVERLAY = '[role="dialog"], [role="alertdialog"], [role="menu"]';

// 用 key 而不是 code：AZERTY 等布局上 A 键的物理位置不同。
export function isSelectAllShortcut(event: ShortcutKey) {
  return (event.metaKey || event.ctrlKey) && !event.shiftKey && !event.altKey && event.key.toLowerCase() === 'a';
}

/** Esc 取消文件选择。输入法选字中的 Esc 是取消输入，不算。 */
export function isClearSelectionKey(event: ShortcutKey & { isComposing?: boolean }) {
  return event.key === 'Escape' && !event.isComposing && !event.metaKey && !event.ctrlKey && !event.altKey && !event.shiftKey;
}

function isEditableTarget(target: EventTarget | null) {
  return Boolean((target as Element | null)?.closest?.(EDITABLE));
}

/** 应用级（main.tsx 注册一次）：焦点不在可编辑元素上时拦下浏览器默认全选，界面文字不被整片选中。 */
export function preventUiSelectAll(event: ShortcutEvent) {
  if (isSelectAllShortcut(event) && !isEditableTarget(event.target)) event.preventDefault();
}

/**
 * 文件管理页可见、焦点不在文件列表也不在可编辑元素上时（点过工具栏、空白处）：Cmd+A 全选文件，
 * Esc 取消选择。焦点在文件列表上时由 FilePane 自己处理；对话框、菜单打开时 Esc 归它们（关闭），这里都跳过。
 */
export function useFileSelectionFallback(enabled: boolean, handlers: { selectAll: () => void; clearSelection: () => void }) {
  const callbacks = useRef(handlers);
  callbacks.current = handlers;
  useEffect(() => {
    if (!enabled) return;
    const onKeyDown = (event: KeyboardEvent) => {
      const selectAll = isSelectAllShortcut(event);
      if ((!selectAll && !isClearSelectionKey(event)) || isEditableTarget(event.target)) return;
      if ((event.target as Element | null)?.closest?.('[data-file-list]')) return;
      // Esc 时 Radix 在捕获阶段已把菜单/对话框关掉并移出 DOM，查 document 会落空；
      // 按键目标仍在（已脱离的）菜单/对话框子树里，沿 closest 能认出来。
      if ((event.target as Element | null)?.closest?.(OVERLAY) || document.querySelector(OVERLAY)) return;
      if (selectAll) callbacks.current.selectAll();
      else callbacks.current.clearSelection();
    };
    document.addEventListener('keydown', onKeyDown);
    return () => document.removeEventListener('keydown', onKeyDown);
  }, [enabled]);
}
