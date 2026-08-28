import test from 'node:test';
import assert from 'node:assert/strict';
import { access, readFile } from 'node:fs/promises';
const root = new URL('../', import.meta.url); const read = file => readFile(new URL(file, root), 'utf8');
const source = Object.fromEntries(await Promise.all(Object.entries({ fileManager: 'ui/src/features/files/FileManager.tsx', filePane: 'ui/src/features/files/FilePane.tsx', fileList: 'ui/src/features/files/useFileList.ts', transfer: 'ui/src/features/files/useTransfer.ts', connection: 'ui/src/features/connections/useConnections.ts' }).map(async ([key, file]) => [key, await read(file)])));
test('phase 7 and 8 remove the vanilla file and connection trees', async () => { for (const file of ['assets', 'views', 'build-renderer.cjs']) await assert.rejects(access(new URL(file, root))); });
test('React file manager keeps the shared panes and hooks', () => { assert.match(source.filePane, /export function FilePane/); assert.match(source.fileManager, /<FilePane[\s\S]*kind="local"/); assert.match(source.fileManager, /<FilePane[\s\S]*kind="remote"/); assert.match(source.fileList, /\$\{sessionId\}:\$\{path\}/); assert.match(source.transfer, /onDownloadProgress|onUploadProgress/); });
test('React file manager avoids manual row DOM work', () => { for (const file of ['filePane', 'fileManager', 'fileList', 'transfer']) assert.doesNotMatch(source[file], /appendChild|querySelector\(['"]#(?:local|remote)-files/); });
test('phase 8 preserves all atomic activation paths', () => { assert.equal((source.connection.match(/原子激活：缓冲快照由后端作为首个 ssh:data 事件发出/g) || []).length, 3); assert.equal((source.connection.match(/getSessionBuffer/g) || []).length, 1); });
