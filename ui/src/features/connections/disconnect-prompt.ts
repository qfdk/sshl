// 所有断开入口（顶栏、会话标签 ×、侧栏图标、右键菜单）共用的确认文案。
export function disconnectPrompt(name: string, { transferring, onFileTab }: { transferring: boolean; onFileTab: boolean }) {
  const consequence = transferring ? '正在进行的文件传输会中断。' : onFileTab ? '文件管理会一并关闭。' : '';
  return {
    title: '断开连接',
    message: [`确定要断开「${name}」吗？`, consequence].filter(Boolean).join('\n'),
    confirmLabel: '断开',
  };
}
