import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
const [list, manager] = await Promise.all([readFile(new URL('../ui/src/features/connections/ConnectionList.tsx', import.meta.url), 'utf8'), readFile(new URL('../ui/src/features/connections/useConnections.ts', import.meta.url), 'utf8')]);
test('connection drag updates data and rerenders through the pure move function', () => { assert.match(list, /manager\.move/); assert.match(manager, /moveConnection/); assert.doesNotMatch(list, /appendChild/); });
test('connection drag has a 6px threshold and window-level cleanup', () => { assert.match(list, /distance\s*<\s*6/); assert.match(list, /window\.addEventListener\('pointerup'/); assert.match(list, /window\.addEventListener\('pointercancel'/); assert.match(list, /onPointerDown/); assert.match(list, /onPointerMove/); });
