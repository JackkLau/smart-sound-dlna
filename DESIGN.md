---
version: alpha
name: 小米音响本地音乐
description: "A Tauri desktop music library and DLNA speaker control surface"
colors:
  primary: "#c74600"
  primary-hover: "#a93b00"
  accent: "#ff6900"
  tint: "#fff1e8"
  background: "#f5f6f8"
  surface: "#ffffff"
  inset: "#eef0f3"
  ink: "#24272d"
  muted: "#626975"
  faint: "#697381"
  line: "#dfe3e8"
  line-strong: "#b9c0ca"
  success: "#387655"
  danger: "#b13e37"
  warning-bg: "#fff7e9"
  focus: "#176ac2"
typography:
  body:
    fontFamily: '"PingFang SC", "Microsoft YaHei", -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif'
    fontSize: "14px"
    lineHeight: "1.6"
  display:
    fontFamily: '"Avenir Next", "PingFang SC", "Microsoft YaHei", sans-serif'
  numeric:
    fontFamily: '"SFMono-Regular", Consolas, monospace'
rounded:
  control: "9px"
  panel: "18px"
  dialog: "20px"
spacing:
  unit: "4px"
  page: "40px"
  page-max: "1400px"
components:
  button:
    minHeight: "42px"
  player:
    minHeight: "116px"
  dialog:
    maxWidth: "480px"
---

# 小米音响本地音乐 Design System

## Overview

A Windows/macOS desktop control surface for sending a local music collection to a DLNA speaker. The current implementation uses a Rust backend and Tauri 2 native shell, with a static HTML/CSS/JavaScript renderer. The visual reference is a silver perforated speaker beside a quiet music-library window: cool paper-white surfaces, charcoal labels, and one orange status light. This is a working single-screen tool, not a marketing page. The brief and agreed Tauri IPC contract are the authority. The established Python browser interface supplied the visual and interaction reference; it is retained unchanged as legacy source, not shipped as a second current surface.

The audience is Chinese-speaking people using a computer on the same local network as their speaker. The supported UI locale is `zh-CN`; the locale alone does not imply a regulated market. No Japanese-market or Japanese-data flows apply. The application does not claim Xiaomi affiliation or model compatibility that device discovery has not established.

The signature is the CSS speaker illustration with a restrained perforated grille and a connection light. Controls, playback information, and error recovery remain conventional. Avoid dashboard metric cards, marketing hero sections, fake album art, fake music/device content, and remote font/CDN dependencies.

Runtime token ownership is Model B: `frontend/style.css` is the canonical runtime owner for the desktop application; this document mirrors accepted values and explains their use. The legacy `xiaomi_dlna/static/style.css` is frozen historical source; desktop changes are owned by `frontend/`. No framework theme adapter or additional styling system is introduced. When tokens change, update both files together. The product offers one light theme plus system forced-colors support.

## Colors

The 6 leading identity colors are background `#f5f6f8`, surface `#ffffff`, ink `#24272d`, muted `#626975`, accent `#ff6900`, and line `#dfe3e8`. The brighter Xiaomi-like orange is decorative only. Primary actions use the darker orange `#c74600` so small white labels retain contrast. Selection uses a warm tint plus pressed-state semantics and an icon. Success and error have text labels; color never carries status alone. Blue focus outlines remain distinct from orange selection. The decorative speaker's metallic gradient values are illustration-specific, not general interface tokens.

## Typography

Display headings use the locally available Avenir Next with Chinese-capable PingFang/Microsoft YaHei fallbacks, 27–32px for the library title and 18–20px for the selected speaker. Body controls use the OS Chinese sans stack, a 14px base, and generous Chinese line height. Secondary metadata is 10–12px, never the only label for an action. Numeric time, file size, and pagination use the local monospace stack for stable widths. Titles and filenames wrap; the player title may scroll within its reserved two-line region. No text relies on hover for full-value access.

## Layout

Desktop: a 280px device column beside a flexible music collection, with 32px between them and a fixed bottom player. The page max width is 1400px. A small laptop narrows the device column and lets import buttons wrap. Below 620px, the device section becomes a compact summary above the music library and playback controls use three rows. The document owns vertical scrolling; only dialog content and unusually long player names scroll internally. No viewport-height constraint is applied to the page shell.

The track list has a 398px minimum surface, 20 records per page, an explicit range and previous/next controls. On small screens the size column is omitted while title, format, and play action remain. The current search and page are restored through guarded sessionStorage in the native window. Desktop state is not shareable navigation, so this is an intentional exception to URL-state defaults. Storage failure leaves working in-memory state. The fixed player always has matching document bottom padding and scroll-padding; narrow screens include safe-area space.

## Elevation & Depth

Use subtle borders and tonal surfaces for library/device hierarchy. Only the playback dock, transient status toast, and dialogs receive subtle shadows. Background blur is reserved for a native dialog backdrop. The CSS speaker has illustration-only shadows to describe its shape; do not reuse them on records or controls.

## Shapes

Controls use a 9px radius, device panels 18px, and dialogs 20px. The smaller library surface uses 12px as a documented content-frame variant. Play is a circular transport button. Other transport icons occupy real 34px buttons; dense mobile player controls are at least 24px. Avoid arbitrary pill badges and decorative nested cards.

## Components

### Runtime token mapping

| Document path | Runtime owner | Consumers |
| --- | --- | --- |
| `colors.*` | matching `--color-*` variables in `style.css` | every global component recipe |
| `typography.body.fontFamily` | `--font-body` | body, form controls, explanatory text |
| `typography.display.fontFamily` | `--font-display` | library and dialog titles |
| `typography.numeric.fontFamily` | `--font-numeric` | playback time, file metadata, pagination |
| `rounded.control` / `panel` / `dialog` | `--radius-control` / `--radius-panel` / `--radius-dialog` | buttons/fields, device panel, dialog |
| `spacing.unit` / `page` / `page-max` | `--space-unit` / `--space-page` / `--page-max` | spacing rhythm, page/dock alignment, maximum width |
| scrollbar semantic colors | `--scroll-thumb`, `--scroll-track`, `--scroll-hover`, `--scroll-active` | global standards and WebKit scrollbar recipes |

### Canonical UI Map

| Capability | Canonical owner | Source of truth | Allowed variants | Verification |
| --- | --- | --- | --- | --- |
| Select/Listbox | native `select#mode` | this contract and `app.js` | native OS popup accepted | keyboard and opened popup |
| Form | `submitDialog`, `fieldError`, shared text-input recipe | agreed Tauri IPC commands and this contract | device URL only; file and folder selection is native | invalid, pending, success, error and IME checks |
| Scrollbar | global `style.css` selectors | runtime tokens and this document | dialog geometry only | static audit and computed global style |
| Toast | shared `announce` function and `#toast` | this contract | success/info | polite live region, one fixed slot |
| CRUD | `mutate`, `acceptState`, and `importMusic` | agreed IPC contract | import/select/control; no delete capability | native backend and desktop full-flow validation |

This single screen has no destructive actions or separate detail workflows; a second UX-CONTRACT.md is intentionally unnecessary. The contract is collected here so owners remain unambiguous.

### Shared interaction contract

All clickable controls are native buttons, links, inputs, select, or dialog. Enabled buttons define hover, pressed, focus-visible, disabled and busy states. Busy requests preserve label dimensions and disable duplicate control submissions. Native select popup geometry is OS-owned; no custom popup is promised. Native modal dialogs own focus trapping, inert background, Escape and restoration. During a submitted device request, the authored dialog stays open until the native operation settles and cannot be dismissed. Before submission, cancellation preserves entered values for reopening. Native file and folder dialogs are explicitly OS-owned and are opened by Rust commands; choosing Cancel returns null and makes no import claim. This is not a browser alert/confirm/prompt substitution. Standard OS window chrome owns window controls; the renderer does not invent a draggable titlebar or emulate platform controls.

Forms use `novalidate`, linked labels/help/errors, first-error focus and preserved values. Search is local and immediate, composition-safe, clearable, and page-clamped. No remote search or debounce is needed. List records and device options are rendered only when their data changes, preserving active controls across background polling.

The Rust backend supplies every device, track and playback state through `window.__TAURI__.core.invoke`. No fetch, HTTP control token, browser file upload, JavaScript filesystem path access or external script is used. The Tauri shell enables `withGlobalTauri`; assets are relative `./style.css` and `./app.js` within `frontend/`. Opening the page outside the desktop app displays a persistent instruction to use the client and leaves native actions disabled.

Initial, empty, no-results, disconnected, failed, transition and confirmed states have distinct copy. Native commands are never automatically replayed. IPC cannot be aborted: slow operations display status and keep mutation protection until the original invocation settles. `get_state` polls every two seconds without overlap, and generations reject results that began before a mutation. Focused seek/volume controls are not reset by polling. A successful action accepts returned backend state; subsequent polling reconciles cached speaker status.

The backend discovers devices on startup and every 30 seconds. `discovering` and `discovery_error` drive honest busy/failure copy; the renderer does not infer discovery success from a timer. A manual rescan uses `discover_devices`. Only backend-provided `is_xiaomi` and `is_xiaomi_pro` create Xiaomi/Pro badges; Pro devices are presented first, followed by other Xiaomi and generic DLNA speakers. Device selection remains explicit.

`pick_music_files` and `pick_music_folder` open OS pickers. Import references local originals, displays a native-selection/waiting state and then confirmed `{added,total,skipped,warnings}`; cancellation hides pending feedback without claiming success. There is no browser upload queue, file-size transfer limit, cloud upload, or unverified drag-and-drop API. The formats and collection limit come from `get_config`. Import failures remain inline with a reselect/retry path. Playback is disabled until a speaker is selected, with an explicit nearby reason.

### Desktop IPC contract

| Renderer operation | Native command | Arguments / result |
| --- | --- | --- |
| Configuration | `get_config` | formats, max_tracks, platform |
| Current state | `get_state` | tracks, devices, selected/current IDs, transport, position/duration/volume, errors/warnings, queue_mode, discovering/discovery_error, interfaces/media_port |
| Manual rediscovery | `discover_devices` | returns state |
| Add by URL | `add_device` | `{url}`; returns state |
| Speaker selection | `select_device` | `{deviceId}`; returns state |
| Play track | `play_track` | `{trackId}`; returns state |
| Transport settings | `control` | `{action,value}`; returns state; value null if absent |
| Local import | `pick_music_files`, `pick_music_folder` | ImportSummary or null on user cancellation |

All controls remain Chinese-localized on Windows and macOS. Native chooser geometry, system title bar and select popup are OS-owned; button/form/status behavior and theme tokens remain application-owned. No event plugin or extra JavaScript package is required.

### Iconography and motion

Locally defined 24px SVG symbols use consistent 1.7px rounded strokes. Icon-only controls have Chinese names and native title hints. Text labels remain on all import and setup actions. Motion is limited to busy discovery/transition signals and 150ms state feedback. Reduced-motion preferences suppress animations and transitions.

### Content voice

Use brief practical Chinese: 添加音乐, 导入文件夹, 重新搜索音响, 继续播放. Avoid marketing claims, protocol trivia in routine controls, arbitrary demo songs, or promises that every Xiaomi speaker supports DLNA. Setup copy explains the supported-DLNA requirement and same-network requirement. Desktop lifecycle copy explicitly says the client must remain running during playback. Product copy never suggests opening a separate browser or running a separate Python service.

## Do's and Don'ts

- Keep import, device connection and playback visible in a single workflow.
- Preserve keyboard focus and in-progress edits during polling.
- Keep server errors and corrective instructions persistent and accessible.
- Do not invent connected devices, playback success or library contents.
- Do not add external assets, inline event handlers, native alert/confirm/prompt flows, or unbounded rendered track lists.

### Migration record

The desktop migration preserves palette, type, spacing, speaker illustration and transport layout. Intentional workflow changes are Rust-owned native file/folder selection, Tauri IPC, automatic discovery plus truthful Pro classification, session-local view restoration, and client-running lifecycle text. The old Python files are not edited and do not define current desktop behavior.

### Voice and Bilibili extension contract

Two existing-style dialogs extend the library: “小爱语音” binds one cloud speaker to the explicitly selected DLNA device, and “Bilibili 导入” previews a video and chooses one part before download. Native selects, shared form/field styles, the shared `announce` live-region helper and native dialog focus management remain canonical. Feature requests use a shared `featureAction` wrapper with scoped errors, pending labels, duplicate protection and IME-safe forms. Background download status remains visible outside the dialog; closing a dialog does not cancel a job. The download list has explicit cancel/retry controls, byte progress only when a server total is known, and truthful queued/downloading/importing/completed/failed/cancelled states. Source links and credentials never become executable HTML.

Login uses a separate ephemeral native webview of the provider's official site, without local IPC capabilities. The user explicitly clicks “完成登录” to import only provider-specific session cookies; no password is entered into the music application. Advanced session entry is a masked field, never persisted or echoed. Account sessions and voice bindings are memory-only, require reconnection after restart, and can be disconnected explicitly. Login failure leaves local playback available. Voice polling accepts only fresh “本地” commands, deduplicates records, stops on repeated errors, and reports the last command result; it is not an always-successful or official Xiaomi integration. No general LAN control endpoint is exposed.

Bilibili audio downloads are a Rust-owned queue (one transfer at a time), bounded in size and published atomically into the application data directory. Only completed, validated AAC/MP4 files enter the library. Existing local imports still reference originals. A source record is optional in the existing index, preserving backward compatibility. Anonymous public videos are supported, with optional ephemeral Bilibili login for authorized content; restricted, unavailable or unsupported media reports a recoverable error. No automatic playlist expansion, audio transcoding, external Python or shell execution is introduced.
