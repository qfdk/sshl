#!/usr/bin/env node
/**
 * Build the renderer payload for the Tauri app.
 *   1. Mirror ./assets, ./views, and the xterm npm packages into ./src/.
 *   2. Render views/index.ejs to ./src/index.html with Tauri-friendly paths
 *      (no app:// protocol, no /node_modules, ipc-bridge.js injected before main).
 *   3. Patch terminal-manager.js's hard-coded app:// xterm paths to use vendor/.
 */
const fs = require('fs');
const path = require('path');
const ejs = require('ejs');

const ROOT = __dirname;
const OUT_DIR = path.join(ROOT, 'src');
const NODE_MODULES = path.join(ROOT, 'node_modules');

function rmrf(p) {
  fs.rmSync(p, { recursive: true, force: true });
}

function cpDir(src, dst) {
  fs.cpSync(src, dst, { recursive: true });
}

// 1. Refresh derived directories
for (const sub of ['assets', 'views', 'vendor']) {
  rmrf(path.join(OUT_DIR, sub));
}

cpDir(path.join(ROOT, 'assets'), path.join(OUT_DIR, 'assets'));
cpDir(path.join(ROOT, 'views'), path.join(OUT_DIR, 'views'));

fs.mkdirSync(path.join(OUT_DIR, 'vendor/xterm/lib'), { recursive: true });
fs.mkdirSync(path.join(OUT_DIR, 'vendor/xterm/css'), { recursive: true });
fs.mkdirSync(path.join(OUT_DIR, 'vendor/xterm-addon-fit/lib'), { recursive: true });
fs.mkdirSync(path.join(OUT_DIR, 'vendor/xterm-addon-canvas/lib'), { recursive: true });
fs.copyFileSync(path.join(NODE_MODULES, 'xterm/lib/xterm.js'), path.join(OUT_DIR, 'vendor/xterm/lib/xterm.js'));
fs.copyFileSync(path.join(NODE_MODULES, 'xterm/css/xterm.css'), path.join(OUT_DIR, 'vendor/xterm/css/xterm.css'));
fs.copyFileSync(path.join(NODE_MODULES, 'xterm-addon-fit/lib/xterm-addon-fit.js'), path.join(OUT_DIR, 'vendor/xterm-addon-fit/lib/xterm-addon-fit.js'));
fs.copyFileSync(path.join(NODE_MODULES, 'xterm-addon-canvas/lib/xterm-addon-canvas.js'), path.join(OUT_DIR, 'vendor/xterm-addon-canvas/lib/xterm-addon-canvas.js'));

// 2. Patch terminal-manager.js's hard-coded app:// xterm references
const tmPath = path.join(OUT_DIR, 'assets/js/terminal-manager.js');
const tmSrc = fs.readFileSync(tmPath, 'utf8').replace(/app:\/\/node_modules\//g, 'vendor/');
fs.writeFileSync(tmPath, tmSrc);

// 3. Render EJS → index.html with Tauri-friendly paths and ipc-bridge.js injection
let html = ejs.render(
  fs.readFileSync(path.join(OUT_DIR, 'views/index.ejs'), 'utf8'),
  { title: 'SSHL', cssBundle: null, rendererScript: null, connections: [] },
  { views: [path.join(OUT_DIR, 'views')], filename: path.join(OUT_DIR, 'views/index.ejs') }
);
html = html.replace(/app:\/\//g, '');
html = html.replace(/(["'])\/?node_modules\//g, '$1vendor/');
html = html.replace(
  /<script type="module" src="([^"]+)"><\/script>/,
  `<script src="assets/js/ipc-bridge.js"></script>\n<script type="module" src="$1"></script>`
);

fs.writeFileSync(path.join(OUT_DIR, 'index.html'), html);
console.log('[build-renderer] OK');
