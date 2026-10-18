import type { FileEntry, FileKind } from './useFileList';

export type CompletionQuery = {
  // 输入框里最后一个分隔符及之前的部分，补全结果拼在它后面
  base: string;
  // 要列出的目录，与文件列表加载时的路径写法一致
  directory: string;
  // 正在输入的名字
  prefix: string;
};

// /usr/lib 这类目录有上千个子目录，下拉框只放前 100 个，继续输入会自然缩小范围。
const SUGGESTION_LIMIT = 100;

// 没有分隔符的相对路径无从列目录，不补全。
export function splitCompletionPath(value: string, kind: FileKind): CompletionQuery | null {
  const index = kind === 'remote' ? value.lastIndexOf('/') : Math.max(value.lastIndexOf('/'), value.lastIndexOf('\\'));
  if (index < 0) return null;
  const base = value.slice(0, index + 1);
  const prefix = value.slice(index + 1);
  if (kind === 'remote') {
    const directory = `/${base}`.replace(/\/+/g, '/').replace(/(.)\/$/, '$1');
    return { base, directory, prefix };
  }
  const trimmed = base.replace(/[\\/]+$/, '');
  // 根目录保留分隔符（"/"、"C:\"），与 isRootPath 的写法一致
  const directory = !trimmed || /^[A-Za-z]:$/.test(trimmed) ? base.slice(0, trimmed.length + 1) : trimmed;
  return { base, directory, prefix };
}

// 只列目录：地址栏是用来跳转的。隐藏目录和 shell 一样，只在输入了 "." 之后才出现，否则 home 目录里一屏全是 .xxx。
export function matchDirectories(entries: FileEntry[], prefix: string): FileEntry[] {
  const needle = prefix.toLowerCase();
  return entries
    .filter(entry => entry.isDirectory && (needle.startsWith('.') || !entry.name.startsWith('.')) && entry.name.toLowerCase().startsWith(needle))
    .sort((a, b) => a.name.localeCompare(b.name))
    .slice(0, SUGGESTION_LIMIT);
}

function commonNamePrefix(names: string[]) {
  let end = names[0].length;
  for (const name of names.slice(1)) {
    let index = 0;
    while (index < end && index < name.length && name[index].toLowerCase() === names[0][index].toLowerCase()) index += 1;
    end = index;
  }
  return names[0].slice(0, end);
}

// Tab：高亮项或唯一匹配补全成目录并带上分隔符，接着就能列下一层；多个匹配时补到公共前缀。
export function tabCompletion(query: CompletionQuery, matches: FileEntry[], highlighted: number): string | null {
  const pick = matches[highlighted] ?? (matches.length === 1 ? matches[0] : null);
  if (pick) return `${query.base}${pick.name}${query.base.slice(-1)}`;
  if (!matches.length) return null;
  const common = commonNamePrefix(matches.map(entry => entry.name));
  return common.length > query.prefix.length ? `${query.base}${common}` : null;
}
