import { createRoot } from 'react-dom/client';
import { App } from './App';
import './index.css';
import '../../assets/css/fonts.css';
import '../../assets/css/main.css';
import '../../assets/css/file-manager.css';
import '../../assets/css/terminal.css';
import '../../assets/css/buttons.css';
import 'xterm/css/xterm.css';
import '../../assets/css/app-runtime.css';

const root = document.getElementById('root');

if (!root) {
  throw new Error('React root element was not found');
}

createRoot(root).render(
  <App />,
);
