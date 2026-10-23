import test from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { remoteDeleteCommands } from '../ui/src/features/files/remote-delete.ts';

// 在临时目录里用真实 shell 跑生成的命令（cwd 也在临时目录，注入的命令只会落在这里）。
function runInTempDir(names, run) {
  const dir = mkdtempSync(join(tmpdir(), 'sshl-rm-'));
  try {
    for (const name of names) writeFileSync(join(dir, name), 'x');
    run(dir);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

test('shell metacharacters in remote names delete exactly that file', () => {
  runInTempDir(['a.txt', 'a$b.txt', "it's", 'q"uote', 'tick`date`'], dir => {
    const targets = ['a$b.txt', "it's", 'q"uote', 'tick`date`'].map(name => join(dir, name));
    for (const command of remoteDeleteCommands(targets)) execFileSync('sh', ['-c', command], { cwd: dir });
    for (const target of targets) assert.equal(existsSync(target), false, `${target} should be deleted`);
    // 双引号拼接时 $b 展开成空，删掉的会是 a.txt。
    assert.equal(existsSync(join(dir, 'a.txt')), true, 'a.txt must survive');
  });
});

test('command substitution in a remote name is not executed', () => {
  // cwd 与目标目录分开：注入的 touch 若被执行，会在 cwd 留下 injected（目标目录可能被整个误删，不能在那里检查）。
  const cwd = mkdtempSync(join(tmpdir(), 'sshl-cwd-'));
  try {
    runInTempDir(['$(touch injected)'], dir => {
      for (const command of remoteDeleteCommands([join(dir, '$(touch injected)')])) execFileSync('sh', ['-c', command], { cwd });
      assert.equal(existsSync(join(cwd, 'injected')), false, 'injected command ran');
      assert.equal(existsSync(join(dir, '$(touch injected)')), false);
      assert.equal(existsSync(dir), true, 'the parent directory must not be deleted');
    });
  } finally {
    rmSync(cwd, { recursive: true, force: true });
  }
});

test('directories are removed recursively and dash-prefixed names are not options', () => {
  const dir = mkdtempSync(join(tmpdir(), 'sshl-rm-'));
  try {
    mkdirSync(join(dir, 'sub', 'deep'), { recursive: true });
    writeFileSync(join(dir, 'sub', 'deep', 'f'), 'x');
    writeFileSync(join(dir, '-rf'), 'x');
    for (const command of remoteDeleteCommands([join(dir, 'sub')])) execFileSync('sh', ['-c', command], { cwd: dir });
    for (const command of remoteDeleteCommands(['-rf'])) execFileSync('sh', ['-c', command], { cwd: dir });
    assert.equal(existsSync(join(dir, 'sub')), false);
    assert.equal(existsSync(join(dir, '-rf')), false);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test('many paths share one command, split before the SSH packet size limit', () => {
  assert.deepEqual(remoteDeleteCommands([]), []);
  assert.equal(remoteDeleteCommands(['/a', '/b', '/c']).length, 1);

  const paths = Array.from({ length: 2000 }, (_, i) => `/srv/data/${'x'.repeat(40)}-${i}`);
  const commands = remoteDeleteCommands(paths);
  assert.ok(commands.length > 1);
  for (const command of commands) {
    assert.ok(command.startsWith('rm -rf -- '));
    assert.ok(command.length <= 30000, `command too long: ${command.length}`);
  }
  // 每个路径恰好出现一次，顺序不变。
  const quoted = commands.flatMap(command => command.slice('rm -rf -- '.length).split(' '));
  assert.deepEqual(quoted, paths.map(path => `'${path}'`));
});
