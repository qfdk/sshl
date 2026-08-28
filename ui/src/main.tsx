import { createRoot } from 'react-dom/client';
import './lib/ipc';
import { App } from './App';
import './index.css';
import 'xterm/css/xterm.css';

const root = document.getElementById('root');
if (!root) throw new Error('React root element was not found');
createRoot(root).render(<App />);
