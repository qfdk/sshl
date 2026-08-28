# CLAUDE.md

Guidance for Claude Code when working in this repo.

## Project Overview

Tauri-based desktop SSH client with a React + TypeScript frontend, xterm.js terminal
emulation, and SFTP file transfer. The backend is Rust (russh + russh-sftp).

## Layout

```
ui/                   Vite React application; shadcn components and Tailwind styles
ui/src/features/      connections, terminal, and file-manager features
ui/src/lib/           IPC bridge, app state, connection ordering, session state
ui/src/assets/fonts/  bundled JetBrainsMono Nerd Font files
src-tauri/            Tauri commands and SSH/SFTP backend
```

## Build & Run

```bash
pnpm install
pnpm vite              # Vite dev server
pnpm vite build        # renderer build
pnpm dev               # Tauri dev
pnpm build             # Tauri application build
```

## Key invariants

- **IPC contract** lives in `ui/src/lib/ipc.ts`. Tauri raw invokes are wrapped to match
  `{success, ...payload}`; `getHomeDir` and other explicitly raw list calls retain their
  raw return values. The bridge imports `invoke`/`listen` directly and does not read
  `window.__TAURI__`.
- **First-paint protocol (atomic activation)** — `ssh_connect` returns a sessionId;
  shell output stays buffered server-side until the renderer calls `ssh_activate_session`.
  The backend then emits the buffered snapshot as the first `ssh:data` event, so the
  snapshot and all subsequent output remain ordered. The renderer must not pull the
  snapshot with `getSessionBuffer` before activating. The sole call site is the
  `terminalResult.isNew` replay path in `ui/src/features/connections/useConnections.ts`.
- **Warm pool** is backend-owned. The frontend makes no prewarm call.
- **SFTP channel reuse** — one cached SFTP session is shared per SSH session.
- **Terminal timing** — `ui/src/features/terminal/useTerminal.ts` keeps CanvasAddon after
  `term.open()`, Unicode11 registration, bottom alignment, ANSI background sampling, and
  PTY resize synchronization. Do not reorder those paths.
- **Bundled Nerd Font** — both JetBrainsMono Nerd Font weights are loaded from
  `ui/src/index.css` and `ui/src/assets/fonts/`; do not replace them with system fallback.
- **Dialog mounting** — Dialogs must not use `forceMount`; `TabsContent` in `App.tsx`
  intentionally keeps `forceMount` so xterm instances survive tab switches.
- **xterm** is pinned to 5.3.0. Do not upgrade to v6.
- **Rust scope** — do not modify `src-tauri/**/*.rs` for frontend migration work.
