import test from 'node:test';
import assert from 'node:assert/strict';
import { matchDirectories, splitCompletionPath, tabCompletion } from '../ui/src/features/files/path-completion.ts';

const dir = name => ({ name, path: `/x/${name}`, isDirectory: true });
const file = name => ({ name, path: `/x/${name}`, isDirectory: false });

test('splits the typed path into the directory to list and the name being typed', () => {
  assert.deepEqual(splitCompletionPath('/Users/qfdk/Doc', 'local'), { base: '/Users/qfdk/', directory: '/Users/qfdk', prefix: 'Doc' });
  assert.deepEqual(splitCompletionPath('/Users/', 'local'), { base: '/Users/', directory: '/Users', prefix: '' });
  assert.deepEqual(splitCompletionPath('/', 'local'), { base: '/', directory: '/', prefix: '' });
  assert.deepEqual(splitCompletionPath('/us', 'remote'), { base: '/', directory: '/', prefix: 'us' });
  assert.deepEqual(splitCompletionPath('C:\\Users\\qf', 'local'), { base: 'C:\\Users\\', directory: 'C:\\Users', prefix: 'qf' });
  assert.deepEqual(splitCompletionPath('C:\\', 'local'), { base: 'C:\\', directory: 'C:\\', prefix: '' });
});

test('remote directories are normalized the way the file list loads them', () => {
  assert.equal(splitCompletionPath('//var//lo', 'remote').directory, '/var');
  assert.equal(splitCompletionPath('var/lo', 'remote').directory, '/var');
  // 远程路径里的反斜杠是普通字符，不是分隔符
  assert.equal(splitCompletionPath('/srv/a\\b', 'remote').prefix, 'a\\b');
});

test('a path without any separator has nothing to list', () => {
  assert.equal(splitCompletionPath('', 'local'), null);
  assert.equal(splitCompletionPath('Users', 'remote'), null);
});

test('suggests only directories starting with the prefix, ignoring case, sorted by name', () => {
  const entries = [dir('Downloads'), file('doc.txt'), dir('Documents'), dir('Desktop'), dir('src')];
  assert.deepEqual(matchDirectories(entries, 'do').map(entry => entry.name), ['Documents', 'Downloads']);
  assert.deepEqual(matchDirectories(entries, '').map(entry => entry.name), ['Desktop', 'Documents', 'Downloads', 'src']);
});

test('hidden directories only show up once the prefix starts with a dot', () => {
  const entries = [dir('.config'), dir('.cache'), dir('code')];
  assert.deepEqual(matchDirectories(entries, '').map(entry => entry.name), ['code']);
  assert.deepEqual(matchDirectories(entries, 'c').map(entry => entry.name), ['code']);
  assert.deepEqual(matchDirectories(entries, '.c').map(entry => entry.name), ['.cache', '.config']);
});

test('tab completes the highlighted or only match into a directory, ready for the next level', () => {
  const query = { base: '/Users/qfdk/', directory: '/Users/qfdk', prefix: 'Do' };
  const matches = [dir('Documents'), dir('Downloads')];
  assert.equal(tabCompletion(query, matches, 1), '/Users/qfdk/Downloads/');
  assert.equal(tabCompletion(query, [dir('Documents')], -1), '/Users/qfdk/Documents/');
  const windows = { base: 'C:\\Users\\', directory: 'C:\\Users', prefix: 'q' };
  assert.equal(tabCompletion(windows, [dir('qfdk')], -1), 'C:\\Users\\qfdk\\');
});

test('tab with several matches extends to their common prefix, or does nothing', () => {
  const query = { base: '/srv/', directory: '/srv', prefix: 'a' };
  assert.equal(tabCompletion(query, [dir('app-api'), dir('app-web')], -1), '/srv/app-');
  assert.equal(tabCompletion({ ...query, prefix: 'app-' }, [dir('app-api'), dir('app-web')], -1), null);
  assert.equal(tabCompletion(query, [], -1), null);
});
