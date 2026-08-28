import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
const [dialog, settings, connectionDialog, app] = await Promise.all(['ui/src/components/ui/dialog.tsx', 'ui/src/layout/SettingsDialog.tsx', 'ui/src/layout/ConnectionDialog.tsx', 'ui/src/App.tsx'].map(file => readFile(new URL(`../${file}`, import.meta.url), 'utf8')));
test('dialogs never use forceMount, while tab panes retain it', () => { assert.doesNotMatch(dialog, /forceMount/); assert.doesNotMatch(connectionDialog, /forceMount/); assert.doesNotMatch(settings, /forceMount/); assert.match(app, /TabsContent[\s\S]*forceMount/); });
test('settings is controlled by React state rather than a vanilla initializer', () => { assert.match(settings, /useSyncExternalStore/); assert.match(settings, /onOpenChange=/); assert.doesNotMatch(settings, /initSettingsUI/); });
