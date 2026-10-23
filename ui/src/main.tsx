import { createRoot } from 'react-dom/client';
import { api } from './lib/ipc';
import { App } from './App';
import './index.css';
import 'xterm/css/xterm.css';
// 必须排在 xterm.css 之后：这些规则覆盖 xterm 自带样式（内边距、层透明、滚动条占位）。
import './features/terminal/terminal.css';

const root = document.getElementById('root');
if (!root) throw new Error('React root element was not found');
createRoot(root).render(<App />);
// 第一屏提交并画出后再撤原生开屏层（src-tauri/src/splash.rs）。
requestAnimationFrame(() => requestAnimationFrame(() => void api.app.ready()));
