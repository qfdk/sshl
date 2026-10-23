// 删除确认文案。多选（Cmd+A 一次可能上百项）只列前几个名字，放在单行 detail 里（超出省略），
// 数量写在标题上，不把整份清单塞进对话框。
const PREVIEW_COUNT = 10;

type DeleteTarget = { path: string; isDirectory?: boolean };

const basename = (path: string) => path.split('/').filter(Boolean).pop() || path;

export function deletePrompt(items: DeleteTarget[]) {
  if (items.length === 1) {
    const [item] = items;
    return {
      title: item.isDirectory ? '删除目录' : '删除文件',
      message: `确定要删除「${basename(item.path)}」${item.isDirectory ? '及其所有内容' : ''}吗？\n此操作不可恢复。`,
      confirmLabel: '删除',
    };
  }
  return {
    title: `删除 ${items.length} 个项目`,
    detail: items.slice(0, PREVIEW_COUNT).map(item => basename(item.path)).join('、'),
    message: '此操作不可恢复。',
    confirmLabel: '删除',
  };
}
