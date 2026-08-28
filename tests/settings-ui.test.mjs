import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const [template, styles, icons] = await Promise.all([
    readFile(new URL('../views/partials/settings-dialog.ejs', import.meta.url), 'utf8'),
    readFile(new URL('../assets/css/settings-dialog.css', import.meta.url), 'utf8'),
    readFile(new URL('../assets/js/icons.js', import.meta.url), 'utf8'),
]);

test('settings navigation provides a rendered icon for every section', () => {
    assert.match(template, /data-settings-tab="font"[\s\S]*data-lucide="type"/);
    assert.match(template, /data-settings-tab="groups"[\s\S]*data-lucide="folder"/);
    assert.match(icons, /'type':\s*'/);
});

test('hidden settings panels stay hidden until their tab is selected', () => {
    assert.match(styles, /\.settings-panel\[hidden\]\s*\{[\s\S]*display:\s*none/);
});

test('settings panels share the same body and bottom action layout', () => {
    assert.equal((template.match(/class="settings-panel-body"/g) || []).length, 2);
    assert.equal((template.match(/class="settings-panel-footer"/g) || []).length, 2);
    assert.match(template, /id="group-manager-save"[^>]*>[\s\S]*保存</);
    assert.match(styles, /\.settings-panel-form[\s\S]*\.settings-panel-footer/);
    assert.match(styles, /\.settings-panel input\[type="text"\][\s\S]*\.settings-panel select/);
    assert.doesNotMatch(styles, /\.settings-panel\s*\{[^}]*height:\s*100%/);
});
