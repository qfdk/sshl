// 文件管理里的拖拽：本地 ⇄ 远程两侧互拖即上传 / 下载，从 Finder 拖进远程一侧即上传。
// 不用 HTML5 拖放：Tauri 的拖放处理器开着（Finder 拖入要靠它拿到本地路径），它把 WKWebView 的拖放回调
// 全部截走、不交给 WebKit，页面内的 drop 事件根本不触发。两侧互拖用指针事件（同会话标签、连接树）。
import type { FileEntry } from './useFileList';

export { releasedMove } from '../../lib/pointer-release';

export type PaneName = 'local' | 'remote';
export type DropTarget = { pane: PaneName; directory: string | null };
export type DropAction = { kind: 'upload' | 'download'; directory: string };

type Located = { closest: (selector: string) => { getAttribute: (name: string) => string | null } | null } | null;

/** 落点元素 → 哪一侧、是否落在某个文件夹行（行上带 data-file-dir）。落在两侧之外为 null。 */
export function dropTargetFrom(element: Located): DropTarget | null {
  const pane = element?.closest('[data-file-pane]')?.getAttribute('data-file-pane');
  if (pane !== 'local' && pane !== 'remote') return null;
  return { pane, directory: element?.closest('[data-file-dir]')?.getAttribute('data-file-dir') || null };
}

/** 同一个落点：同一侧、同一个文件夹行（或都不在文件夹行上）。指针每次移动都得到新对象，内容一样不算换了落点。 */
export function sameDropTarget(a: DropTarget | null, b: DropTarget | null) {
  return a === b || (a !== null && b !== null && a.pane === b.pane && a.directory === b.directory);
}

/** 拖拽中跟着指针的提示：拖了什么、指针在哪、松手会传到哪个目录（不能放为 null）。 */
export type DragPreview = { pane: PaneName; items: FileEntry[]; x: number; y: number; destination: string | null };

// 拖拽浮层的小 store：指针位置每次移动都变，放进 FileManager 的 state 会让整个文件管理（两侧列表）
// 按指针频率重渲染；放这里只有订阅它的浮层重渲染。
export function createDragPreviewStore() {
  let preview: DragPreview | null = null;
  const listeners = new Set<() => void>();
  return {
    get: () => preview,
    set: (next: DragPreview | null) => {
      preview = next;
      listeners.forEach(listener => listener());
    },
    subscribe: (listener: () => void) => {
      listeners.add(listener);
      return () => { listeners.delete(listener); };
    },
  };
}

export type DragPreviewStore = ReturnType<typeof createDragPreviewStore>;

/** 拖到哪、做什么：本地 → 远程 = 上传，远程 → 本地 = 下载，Finder 只能拖进远程。目标目录缺省为那一侧的当前目录。 */
export function planDrop(source: PaneName | 'finder', target: DropTarget | null, current: Record<PaneName, string>): DropAction | null {
  if (!target) return null;
  if (source === 'finder' ? target.pane !== 'remote' : source === target.pane) return null;
  const directory = target.directory || current[target.pane];
  if (!directory) return null;
  return { kind: target.pane === 'remote' ? 'upload' : 'download', directory };
}

/** 按住的行在选中项里就拖整组选中，否则只拖这一行。「..」不能拖。 */
export function dragItems(entry: FileEntry, selected: Set<string>, files: FileEntry[]) {
  if (entry.name === '..') return [];
  return selected.has(entry.path) ? files.filter(file => file.name !== '..' && selected.has(file.path)) : [entry];
}

export function dragLabel(items: FileEntry[]) {
  return items.length === 1 ? items[0].name : `${items.length} 个项目`;
}

/** Tauri 拖放事件的坐标：macOS 上 wry 给的是窗口内的点（即 CSS 像素），其他平台是物理像素。 */
export function dropPointToCss(position: { x: number; y: number }, isMac: boolean, devicePixelRatio: number) {
  return isMac ? { x: position.x, y: position.y } : { x: position.x / devicePixelRatio, y: position.y / devicePixelRatio };
}
