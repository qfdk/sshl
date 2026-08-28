import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
const [database, configStore, bridge, lib] = await Promise.all([readFile(new URL('../src-tauri/src/db.rs', import.meta.url), 'utf8'), readFile(new URL('../src-tauri/src/config_store.rs', import.meta.url), 'utf8'), readFile(new URL('../ui/src/lib/ipc.ts', import.meta.url), 'utf8'), readFile(new URL('../src-tauri/src/lib.rs', import.meta.url), 'utf8')]);
test('group order has an explicit persisted IPC contract', () => { assert.match(database, /CREATE TABLE IF NOT EXISTS connection_groups/); assert.match(configStore, /config_get_connection_groups/); assert.match(configStore, /groups:\s*Option<Vec<String>>/); assert.match(bridge, /getConnectionGroups/); assert.match(bridge, /applyConnectionLayout/); assert.match(lib, /config_store::config_get_connection_groups/); });
