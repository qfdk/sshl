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
  the Tauri global bridge.
- **First-paint protocol (atomic activation)** — `ssh_connect` returns a sessionId;
  shell output stays buffered server-side until the renderer calls `ssh_activate_session`.
  The backend then emits the buffered snapshot as the first `ssh:data` event, so the
  snapshot and all subsequent output remain ordered. The renderer must not pull the
  snapshot with `getSessionBuffer` before activating. The sole call site is the
  `terminalResult.isNew` replay path in `ui/src/features/connections/useConnections.ts`.
- **Terminal flow control** — the shell channel is opened in a manual-window mode added by the vendored
  russh patch (`src-tauri/vendor/`, see its README): the server can send at most 2MB beyond what the
  renderer has acked. Every `ssh:data` carries `bytes`; it must reach xterm through `flowAck.deliver`
  (`ui/src/lib/flow-ack.ts`), which acks via `ssh_ack_data` once xterm has parsed it. Dropping that path
  freezes the terminal after 2MB of output. Other channels (SFTP, exec, probes) keep russh's auto window
  (`CONNECTION_WINDOW` in `ssh.rs`, 8MB; SFTP download segments / in-flight bytes are derived from it in `sftp.rs`).
- **SFTP timeouts** — no per-request deadline that a slow link can trip: `sftp_stall.rs` wraps the channel and
  fails every waiting request only when nothing at all has arrived for `SFTP_STALL` (60s); `sftp_for` then reopens
  the session with the same identity. Downloads write `.<name>.<rand>.sshl-part` next to the target and rename
  on success, so a failure never leaves a truncated file or clobbers the existing one.
- **Warm pool** is backend-owned. The frontend makes no prewarm call.
- **Window reveal** — the main window is built in `create_main_window` (`src-tauri/src/lib.rs`,
  `create: false` in tauri.conf.json) at the screen work area, not via `maximized` (tao's async
  maximize resizes the WKWebView while WebContent is launching and blocks the main thread ~430ms).
  It starts at alpha 0; `ui/src/main.tsx` must call `api.app.ready()` after the first painted frame,
  otherwise the window only appears at the 3s fallback. Do not hide the window instead: a hidden
  WKWebView does not paint or fire rAF, so it would reveal a blank frame. After first paint,
  closing the main window (red button / ⌘W) only hides it and the process stays resident;
  `RunEvent::Reopen` (Dock click) shows it again. ⌘Q quits.
- **SFTP channel reuse** — one cached SFTP session is shared per SSH session.
- **File manager follows the terminal user** — after `su` / `sudo -i` in the terminal, the file
  manager runs as that user (`ui/src/features/files/file-user.ts`, `src-tauri/src/terminal_user.rs`).
  `file_switch_user` replaces the cached SFTP session with an `sftp-server` started via su/sudo;
  `sftp_for` reopens it with the same identity. Remote shell-based file ops (delete, chmod, chown)
  must go through `exec_as_file_user` / `file_shell`, not a plain exec or `ssh_execute`. Only the
  needed user's own saved password is used automatically; never try other saved passwords.
  On any failure before SFTP starts, EOF + close the channel (russh does not close on drop, and a
  waiting `sudo` holds the user's timestamp lock).
- **Drag and drop** — Tauri's drag-drop handler is on (Finder drops need local paths) and swallows
  WKWebView drag callbacks, so in-page HTML5 drop events never fire. Drags inside the app use
  pointer events (`file-drag.ts`, session tabs, connection tree). WKWebView often drops pointerup
  (e.g. the first click after the app is activated), so every pointer drag must handle a move with the
  left button released (`releasedMove` / `pointerMoveStep` in `ui/src/lib/pointer-release.ts`: drop in
  place or cancel, never start a drag) and clear stale press state on the next window pointerdown.
- **Terminal timing** — `ui/src/features/terminal/useTerminal.ts` loads the GPU renderer
  (WebglAddon, falling back to CanvasAddon on failure or context loss) after `term.open()`,
  Unicode11 registration, bottom alignment, ANSI background sampling, and PTY resize
  synchronization. Do not reorder those paths. xterm and its addons are a lazy chunk
  (`loadXterm`); do not import them statically. A hidden spare terminal is prebuilt at idle and
  claimed by `initTerminal`; its fitted size is sent as `cols`/`rows` with `ssh_connect` so the
  PTY opens at the real grid size. Resize is per animation frame; PTY `window_change` is
  deduped and throttled to ~30/s. Release WebGL contexts explicitly on dispose (xterm.js #6068).
- **Bundled Nerd Font** — both JetBrainsMono Nerd Font weights are loaded from
  `ui/src/index.css` and `ui/src/assets/fonts/`; do not replace them with system fallback.
- **Prompts** — use `alertDialog` / `confirmDialog` / `passwordDialog` from `ui/src/lib/dialogs.ts` (rendered by
  `layout/DialogHost`). Never `window.alert` / `window.confirm` / plugin-dialog `ask`: the dialog
  plugin's init script swaps alert/confirm for native NSAlert (English buttons, narrow, breaks Chinese
  sentences mid-line), and its `confirm` returns a Promise, so `if (!confirm(..))` never blocks.
- **Dialog mounting** — Dialogs must not use `forceMount`; `TabsContent` in `App.tsx`
  intentionally keeps `forceMount` so xterm instances survive tab switches.
- **xterm** is pinned to 5.3.0. Do not upgrade to v6.
- **Rust scope** — do not modify `src-tauri/**/*.rs` for frontend migration work.
