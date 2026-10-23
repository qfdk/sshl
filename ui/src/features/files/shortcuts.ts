import { useEffect, useRef } from 'react';

type ShortcutKey = Pick<KeyboardEvent, 'key' | 'metaKey' | 'ctrlKey' | 'shiftKey' | 'altKey'>;

const EDITABLE = 'input, textarea, select, [contenteditable=""], [contenteditable="true"]';

// 用 key 而不是 code：AZERTY 等布局上 A 键的物理位置不同。
export function isSelectAllShortcut(event: ShortcutKey) {
  return (event.metaKey || event.ctrlKey) && !event.shiftKey && !event.altKey && event.key.toLowerCase() === 'a';
}

/**
 * 焦点不在文件列表、也不在可编辑元素上时（点过工具栏、空白处），Cmd+A 默认把整页界面文字全选；
 * 这里拦下改为全选文件。焦点在文件列表上时由 FilePane 自己处理（已 preventDefault，这里跳过）。
 */
export function useSelectAllFallback(enabled: boolean, onSelectAll: () => void) {
  const callback = useRef(onSelectAll);
  callback.current = onSelectAll;
  useEffect(() => {
    if (!enabled) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.defaultPrevented || !isSelectAllShortcut(event)) return;
      if ((event.target as Element | null)?.closest?.(EDITABLE)) return;
      event.preventDefault();
      callback.current();
    };
    document.addEventListener('keydown', onKeyDown);
    return () => document.removeEventListener('keydown', onKeyDown);
  }, [enabled]);
}
