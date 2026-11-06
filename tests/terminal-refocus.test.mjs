import test from 'node:test';
import assert from 'node:assert/strict';
import { shouldRefocusTerminal } from '../ui/src/features/terminal/refocus.ts';

const body = { tagName: 'BODY' };
const input = { tagName: 'INPUT' };
const listDiv = { tagName: 'DIV' };

test('切回终端标签：焦点交给终端（之前留在文件列表 / 标签按钮上，直接打字没反应）', () => {
  assert.equal(shouldRefocusTerminal({ reason: 'tab', activeTab: 'terminal', active: listDiv, dialogOpen: false }), true);
  assert.equal(shouldRefocusTerminal({ reason: 'tab', activeTab: 'terminal', active: body, dialogOpen: false }), true);
});

test('有对话框开着、或不在终端标签：不抢焦点', () => {
  assert.equal(shouldRefocusTerminal({ reason: 'tab', activeTab: 'terminal', active: body, dialogOpen: true }), false);
  assert.equal(shouldRefocusTerminal({ reason: 'tab', activeTab: 'file-manager', active: body, dialogOpen: false }), false);
});

test('窗口重新获得焦点：焦点不在输入框里就交给终端；用户正在某个输入框里就不动', () => {
  assert.equal(shouldRefocusTerminal({ reason: 'window', activeTab: 'terminal', active: body, dialogOpen: false }), true);
  assert.equal(shouldRefocusTerminal({ reason: 'window', activeTab: 'terminal', active: null, dialogOpen: false }), true);
  assert.equal(shouldRefocusTerminal({ reason: 'window', activeTab: 'terminal', active: { tagName: 'BUTTON' }, dialogOpen: false }), true, '焦点停在「终端」标签按钮上');
  assert.equal(shouldRefocusTerminal({ reason: 'window', activeTab: 'terminal', active: { tagName: 'DIV', isContentEditable: true }, dialogOpen: false }), false);
  assert.equal(shouldRefocusTerminal({ reason: 'window', activeTab: 'terminal', active: input, dialogOpen: false }), false);
  assert.equal(shouldRefocusTerminal({ reason: 'window', activeTab: 'file-manager', active: body, dialogOpen: false }), false);
});
