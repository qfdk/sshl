import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
const files = await Promise.all(['ui/src/App.tsx', 'ui/src/layout/ConnectionDialog.tsx', 'ui/src/layout/SettingsDialog.tsx', 'ui/src/layout/Terminal.tsx', 'ui/src/features/files/FileManager.tsx'].map(path => readFile(new URL(`../${path}`, import.meta.url), 'utf8')));
test('useSyncExternalStore never takes an inline subscribe callback', () => { for (const source of files) assert.doesNotMatch(source, /useSyncExternalStore\(\s*\(\s*callback\s*\)\s*=>/); });
test('each subscribing component holds a module-level subscribe constant', () => { for (const source of files.filter(source => source.includes('useSyncExternalStore'))) assert.match(source, /^const subscribe[A-Za-z]+ = \(callback: \(\) => void\) => subscribe\('/m); });
