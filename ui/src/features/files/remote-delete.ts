// 远程删除经 ssh exec 交给远端 shell：路径必须单引号包住（引号内不做任何展开），
// 内嵌单引号用 '\'' 闭合-转义-重开，与 Rust 侧 shell_quote 一致。
function shellQuote(value: string) {
  return `'${value.replace(/'/g, `'\\''`)}'`;
}

// RFC 4253 要求实现至少能处理 32768 字节的包；exec 命令按此分批，全选大目录时不至于被服务端拒收。
const MAX_COMMAND_LENGTH = 30000;

// 多个路径合进尽量少的 `rm -rf --` 命令：删 N 项只开一个 exec 通道，而不是 N 个。
export function remoteDeleteCommands(paths: string[]) {
  const commands: string[] = [];
  let current = '';
  for (const path of paths) {
    const arg = ` ${shellQuote(path)}`;
    if (current && current.length + arg.length > MAX_COMMAND_LENGTH) {
      commands.push(current);
      current = '';
    }
    current = (current || 'rm -rf --') + arg;
  }
  if (current) commands.push(current);
  return commands;
}
