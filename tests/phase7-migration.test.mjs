import test from 'node:test';
import assert from 'node:assert/strict';
import { access, readFile } from 'node:fs/promises';

const root = new URL('../', import.meta.url);
const read = file => readFile(new URL(file, root), 'utf8');

const files = {
  fileManager: 'ui/src/features/files/FileManager.tsx',
  filePane: 'ui/src/features/files/FilePane.tsx',
  fileList: 'ui/src/features/files/useFileList.ts',
  transfer: 'ui/src/features/files/useTransfer.ts',
  main: 'assets/js/main-entry.js',
  ui: 'assets/js/ui-manager.js',
  connection: 'assets/js/connection-manager.js',
  app: 'ui/src/App.tsx',
  index: 'ui/src/main.tsx',
  renderer: 'build-renderer.cjs',
  view: 'views/index.ejs',
  css: 'assets/css/main.css',
};

const source = Object.fromEntries(
  await Promise.all(Object.entries(files).map(async ([key, file]) => [key, await read(file)])),
);

test('phase 7 removes the vanilla file manager assets and partial', async () => {
  for (const file of ['assets/js/file-manager.js', 'assets/css/file-manager.css', 'views/partials/file-manager.ejs']) {
    await assert.rejects(access(new URL(file, root)));
  }
});

test('phase 7 has one shared FilePane and the requested hooks', () => {
  assert.match(source.filePane, /export function FilePane\(\{[\s\S]*kind[\s\S]*path[\s\S]*files/);
  assert.match(source.fileManager, /<FilePane[\s\S]*kind="local"/);
  assert.match(source.fileManager, /<FilePane[\s\S]*kind="remote"/);
  assert.match(source.fileList, /\$\{sessionId\}:\$\{path\}/);
  assert.match(source.fileList, /isDirectory[\s\S]*localeCompare/);
  assert.match(source.transfer, /onDownloadProgress|onUploadProgress/);
});

test('phase 7 removes file-manager responsibilities from vanilla managers', () => {
  assert.doesNotMatch(source.main, /file-manager\.js|setupEnterKeyHandler|setupFileTransferListeners|transfer-progress-bar|pane-divider/);
  assert.doesNotMatch(source.ui, /showFileManagerLoading|showTransferStatus|browse-local|remote-path|local-path|go-remote-path/);
  assert.doesNotMatch(source.index, /file-manager\.css/);
  assert.doesNotMatch(source.renderer, /file-manager\.css/);
  assert.doesNotMatch(source.view, /file-manager\.css|file-manager\.ejs/);
  assert.doesNotMatch(source.css, /\.file-manager|\.split-pane|\.pane-header|\.path-bar|#file-manager-loading|\.transfer-status/);
});

test('phase 7 React file manager avoids manual row DOM work', () => {
  assert.doesNotMatch(source.filePane, /appendChild|querySelector\(['"]#(?:local|remote)-files|addEventListener\(/);
  assert.doesNotMatch(source.fileManager, /appendChild|querySelector\(['"]#(?:local|remote)-files/);
  assert.doesNotMatch(source.fileList, /appendChild|querySelector\(['"]#(?:local|remote)-files|addEventListener\(/);
  assert.doesNotMatch(source.transfer, /appendChild|querySelector\(['"]#(?:local|remote)-files|addEventListener\(/);
});

test('phase 7 preserves atomic activation paths', () => {
  assert.equal((source.connection.match(/原子激活：缓冲快照由后端作为首个 ssh:data 事件发出/g) || []).length, 3);
  assert.equal((source.connection.match(/getSessionBuffer/g) || []).length, 1);
});
