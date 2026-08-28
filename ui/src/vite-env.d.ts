/// <reference types="vite/client" />

import type { Api } from './lib/ipc';

declare global {
  interface Window {
    api: Api;
  }
}

export {};
