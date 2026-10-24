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
  page: "28px"
  page-max: "1400px"
components:
  button:
    minHeight: "40px"
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

Desktop: a 280px device column beside a flexible music collection, with 20px between them and a fixed bottom player. The page max width is 1400px. A small laptop narrows the device column and lets import buttons wrap. Below 620px, the device section becomes a compact summary above the music library and playback controls use three rows. The document owns vertical scrolling; only dialog bodies and unusually long player names scroll internally. No viewport-height constraint is applied to the page shell.

The track list has a 340px minimum surface (320px in the narrow layout), 20 records per page, an explicit range and previous/next controls. On small screens the size column is omitted while title, format, and play action remain. The current search and page are restored through guarded sessionStorage in the native window. Desktop state is not shareable navigation, so this is an intentional exception to URL-state defaults. Storage failure leaves working in-memory state. The fixed player always has matching document bottom padding and scroll-padding; narrow screens include safe-area space.

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
| Form | `submitDialog` / `fieldError` for device setup; `MusicFeatures.onSubmit` / `featureAction` / `errorAt` for feature dialogs; shared text-input recipe | agreed Tauri IPC commands and this contract | device URL, imports, voice and song names; file/folder selection remains native | invalid, pending, success, error, focus and IME checks |
| Scrollbar | global `style.css` selectors | runtime tokens and this document | dialog geometry only | static audit and computed global style |
| Toast | shared `announce` function and `#toast` | this contract | success/info | polite live region, one fixed slot |
| CRUD | `mutate`, `acceptState`, and `importMusic` | agreed IPC contract | import/select/control/rename/remove from library; audio files retained | native backend and desktop full-flow validation |

This single screen has no file deletion or separate detail workflows; a second UX-CONTRACT.md is intentionally unnecessary. The contract is collected here so owners remain unambiguous.

### Shared interaction contract

All clickable controls are native buttons, links, inputs, select, or dialog. Enabled buttons define hover, pressed, focus-visible, disabled and busy states. Busy requests preserve label dimensions and disable duplicate control submissions. Native select popup geometry is OS-owned; no custom popup is promised. Native modal dialogs own focus trapping, inert background, Escape and restoration. During a submitted device request, the authored dialog stays open until the native operation settles and cannot be dismissed. Before submission, cancellation preserves entered values for reopening. Native file and folder dialogs are explicitly OS-owned and are opened by Rust commands; choosing Cancel returns null and makes no import claim. This is not a browser alert/confirm/prompt substitution. Standard OS window chrome owns window controls; the renderer does not invent a draggable titlebar or emulate platform controls.

Forms use `novalidate`, linked labels/help/errors, first-error focus and preserved values. Search is local and immediate, composition-safe, clearable, and page-clamped. No remote search or debounce is needed. List records and device options are rendered only when their data changes, preserving active controls across background polling.

The Rust backend supplies every device, track and playback state through `window.__TAURI__.core.invoke`. No fetch, HTTP control token, browser file upload, JavaScript filesystem path access or external script is used. The Tauri shell enables `withGlobalTauri`; assets are relative `./style.css` and `./app.js` within `frontend/`. Opening the page outside the desktop app displays a persistent instruction to use the client and leaves native actions disabled.

Initial, empty, no-results, disconnected, failed, transition and confirmed states have distinct copy. Native commands are never automatically replayed. IPC cannot be aborted: slow operations display status and keep mutation protection until the original invocation settles. `get_state` polls every two seconds without overlap, and generations reject results that began before a mutation. Focused seek/volume controls are not reset by polling. A successful action accepts returned backend state; subsequent polling reconciles cached speaker status.

The backend discovers devices once on startup; healthy or unselected devices are not periodically rediscovered. Current-device health polling remains active. `device_connection` is `unselected`, `checking`, `connected`, or `disconnected`; only confirmed connections enable playback. Disconnection keeps the selected device visible, offers immediate retry by clicking that device, and retries automatically with a 2/4/8/16/30-second capped backoff. Only while disconnected, rediscovery runs approximately every 30 seconds to refresh the same device ID/address. Recovery never starts playback or selects another device. User actions preempt read-only status queries without cancelling in-flight playback mutations. Optional position/volume reads time out after one second and cannot mark a responsive renderer offline. `discovering` and `discovery_error` drive honest busy/failure copy; the renderer does not infer discovery success from a timer. A manual rescan uses `discover_devices`. Only backend-provided `is_xiaomi` and `is_xiaomi_pro` create Xiaomi/Pro badges; Pro devices are presented first, followed by other Xiaomi and generic DLNA speakers. Device selection remains explicit.

`pick_music_files` and `pick_music_folder` open OS pickers. Import references local originals, displays a native-selection/waiting state and then confirmed `{added,total,skipped,warnings}`; cancellation hides pending feedback without claiming success. There is no browser upload queue, file-size transfer limit, cloud upload, or unverified drag-and-drop API. The formats and collection limit come from `get_config`. Import failures remain inline with a reselect/retry path. Playback is disabled until a speaker connection is confirmed, with an explicit nearby reason.

### Compact controls and dialog scroll ownership

The default density serves repeated library and import operations: 28px desktop page margins, a 72px top bar, 20px workspace gap, 56px minimum track rows, and 40px buttons/text inputs. Narrow layouts use 16px page margins and a 64px top bar. Text sizes, label hierarchy and the existing focus/disabled states remain unchanged. Form sections use 12px separation, with 6–8px gaps inside a group. Empty error regions keep a 20px reserved line to avoid shifting adjacent actions when feedback arrives.

Every authored dialog shares one frame: `.dialog-heading` is a non-scrolling sibling of `.dialog-body`. The open dialog uses a column flex layout bounded by the viewport; only the body has vertical overflow, a stable scrollbar gutter and contained overscroll. The title and close button remain visible at every body scroll position. Header and body use 20px horizontal padding (16px in the narrow layout); body controls have 12px scroll margins so keyboard focus remains clear of the frame edges. The manual-device form is itself the body; feature dialogs wrap their existing forms in the shared body. Primary actions remain part of their form and are reachable by body scrolling. Native modal focus, Escape, pending-dismissal protection, validation and draft behavior remain owned by the existing JavaScript handlers.

### Desktop IPC contract

| Renderer operation | Native command | Arguments / result |
| --- | --- | --- |
| Configuration | `get_config` | formats, max_tracks, platform |
| Current state | `get_state` | tracks, devices, selected/current IDs, device_connection, transport, position/duration/volume, errors/warnings, queue_mode, discovering/discovery_error, interfaces/media_port |
| Manual rediscovery | `discover_devices` | returns state |
| Add by URL | `add_device` | `{url}`; returns state |
| Speaker selection | `select_device` | `{deviceId}`; returns state |
| Play track | `play_track` | `{trackId}`; returns state |
| Transport settings | `control` | `{action,value}`; returns state; value null if absent |
| Local import | `pick_music_files`, `pick_music_folder` | ImportSummary or null on user cancellation |
| Suggest song title | `recognize_track_name` | `{trackId}`; `{title: string or null, source}`; draft only |
| Save song title | `rename_track` | `{trackId,title,expectedTitle}`; returns state; persisted library metadata |
| Remove from library | `remove_track` | `{trackId}`; returns state; retains source/downloaded files |

### Library row playback and removal

A single native `.track-play` button stretches over each list item, covering title, filename, metadata, whitespace and the visible play icon. Its normal keyboard Enter/Space behavior is retained. The content stays legible while the row is unavailable, and the existing speaker/foreground-operation gate disables the whole play target. Secondary `曲名` and `移除` controls are sibling buttons above the play target in a row-local stacking context; they cannot dispatch play by bubbling or by pointer overlap. A 126px action column keeps the visible play icon and two 40px text actions distinct at desktop and narrow widths.

`library-actions.js` reuses `MusicFeatures.onSubmit` / `featureAction` / `errorAt`, `MusicUI.mutate` / `announce`, and the fixed-heading native dialog frame for removal. The confirmation names the track, says original and downloaded audio files remain, explains that custom names must be set again after reimport, and warns that removing the current track stops playback. Cancel receives initial focus; waiting blocks dismissal and duplicates. Errors stay inline with the song still shown, and cancellation or success returns focus to the same row or the next row at that position, falling back to the library heading for an empty result.

Removal only persists an index change and rolls back the record at its original position on save failure. A current-track removal first stops the renderer; stop failure keeps the record. Successful current-track removal clears the current ID, URI and completion evidence without starting another song. Other removals send no playback requests. Removed IDs no longer receive new media requests. Existing search/page state is retained and clamped after removal, including the last item on a page. Explicit local reimport or a fresh Bilibili import can restore the music; completed download history does not substitute for current library membership.

### Song names and voice playback

Each library row retains its play control and adds the text action `曲名`. `track-editor.js` uses the existing native feature-dialog, `MusicFeatures.onSubmit` / `featureAction` / `errorAt` form lifecycle, and `MusicUI.mutate` / `announce` state and feedback owners. It does not introduce a second form or mutation framework. Its 520px dialog is a content-width variant with the existing spacing, color and field tokens. Song naming works without a connected speaker.

Recognition reads local audio title tags, then conservative filename or video-title hints; it neither uploads audio nor claims acoustic fingerprinting. New imports use a suggestion when available. Existing rows change only after Save. Recognition changes the draft and identifies its source; no reliable result keeps the draft. Manual saves persist across restart and reimport and become the voice matching title immediately. Saving does not restart playback or replace the audio file. Names must contain letters or numbers, have at most 120 Unicode characters and no control characters; names equal under voice normalization require a singer/version distinction.

The form keeps a live Chinese example `小爱同学，播放本地 + 曲名`, with a reminder to enable voice reception. Pending actions prevent duplicate submissions and dialog dismissal; failure stays inline with the input preserved and focused. IME composition never submits. Cancel preserves the per-song draft for reopening during this session. Closing returns focus to that song's current row action or the library heading if filtering removed it. Polling cannot overwrite an open draft. An expected-title conflict asks the user to reopen the editor instead of silently overwriting newer data.

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

Two existing-style dialogs extend the library: “小爱语音” binds one cloud speaker to the explicitly selected DLNA device, and “Bilibili 导入” previews videos or paginated favorites and explicitly selects audio parts before download. Native selects, shared form/field styles, the shared `announce` live-region helper and native dialog focus management remain canonical. Feature requests use a shared `featureAction` wrapper with scoped errors, pending labels, duplicate protection and IME-safe forms. Background download status remains visible outside the dialog; closing a dialog does not cancel a job. The download list has explicit cancel/retry controls, byte progress only when a server total is known, and truthful queued/downloading/importing/completed/failed/cancelled states. Source links and credentials never become executable HTML.

Login uses a separate ephemeral native webview of the provider's official site, without local IPC capabilities. The native backend watches only that isolated window’s provider-scoped cookies, exchanges the authenticated Xiaomi account session for a signed micoapi session when the web callback does not issue a token, validates the service session, saves it to the OS credential store, then closes the window and publishes connected state. Waiting, checking, cancellation and errors are reflected by the shared account renderer; “重试连接” retries the visible login window or, when no window is open, the saved session. A ten-minute deadline bounds web monitoring, and close/logout invalidate pending attempts. Parent-domain and HttpOnly cookies are read via the native store on both macOS and Windows; no password is entered into the music application. Advanced session entry is masked, never echoed, and only its validated provider-scoped service cookies may be saved. Login failure leaves local playback available. Voice polling accepts only fresh “本地” commands, deduplicates records, stops on repeated errors, and reports the last command result; it is not an always-successful or official Xiaomi integration. No general LAN control endpoint is exposed.

Bilibili audio downloads are a Rust-owned queue (one transfer at a time), bounded in size and published atomically into the application data directory. Only completed, validated AAC/MP4 files enter the library. Existing local imports still reference originals. A source record is optional in the existing index, preserving backward compatibility. Anonymous public videos are supported, with optional OS-protected Bilibili login for authorized content; restricted, unavailable or unsupported media reports a recoverable error. Favorite folders expand only on explicit user requests; audio transcoding, external Python and shell execution are not introduced.

### Persistent account sessions (0.6.0)

The user explicitly requested login restoration across restarts and upgrades. `SessionCache` owns provider-scoped macOS Keychain / Windows Credential Store access; its stable service identifier has no app-version or install-path component. It never falls back to plaintext or keyring's mock backend. Only micoapi `userId` / `serviceToken` and allowlisted Bilibili cookies are stored; passwords, Xiaomi account `passToken`, signed URLs, the library index and frontend storage remain excluded. The per-provider `LoginFlow` gate serializes restore, login and logout; a late cancelled save is cleared. Logout first removes the stored credential and reports an error if removal fails, so it never falsely promises a persistent logout.

Startup restores both accounts in background tasks without blocking local music. A saved credential is not a connected account: service validation must succeed before publishing connected state. Failed validation keeps retry data and shows a persistent inline error; expiry still requires login. `renderAccounts` remains the single owner for both providers' restoring, saved-but-offline, connected and cleared states. “退出登录” clears that provider only. Voice binding and listening remain explicit, session-local actions and are never silently enabled by account restoration. Validation includes memory-boundary failure/race checks, real OS credential read/write/delete across processes, and shared renderer tests. No visual tokens or dialog/scroll owners change.

Xiaomi account credentials (`passToken`) are sent only to the fixed Xiaomi account endpoint. The STS request is restricted to the exact HTTPS MiNA host and contains the signed challenge, never the account Cookie. Empty authorization failures surface in the main dialog instead of leaving an unexplained waiting state. All challenge material stays in memory.

### Favorites import (0.4.0)

This user-requested extension intentionally replaces the one-part-only import rule. Video and favorites sources share `BilibiliSelection` in `frontend/import-selection.js` and the checkbox picker in `frontend/imports.js`. Native radio inputs own source switching; native checkboxes own individual and per-video all-selection; native select owns the account folder picker. The existing `featureAction`, `errorAt`, `announce`, button, field and dialog owners remain canonical. No visual tokens change.

Favorites use server pagination of 20 videos, explicit previous/next buttons, total/page status, and unavailable/empty/error states. Selections survive paging and switching sources within this dialog session; a successfully opened different folder clears its previous selection. Metadata is bounded and refreshed explicitly. Both sources submit at most 100 selected CIDs; accepted tracks are cleared, skipped tracks retain selection and have persistent per-track reasons. Clearing selection does not delete music. Foreground import requests lock the source and selection until completion, with no silent replay. Download workers remain sequential and keep up to 200 recent job records. Closing the dialog retains view state and does not stop downloads.

| Capability | Canonical owner | Source of truth | Allowed variants | Verification |
| --- | --- | --- | --- | --- |
| Table Selection | `BilibiliSelection` and shared audio picker | this favorites contract | per-part, all parts of one video; cross-page retention | `tests/import-selection.test.cjs`, native app acceptance |

Reconcile drift: the prior single-part / no-playlist behavior changes under the explicit favorites request; token values, typography, native select ownership, notifications and dialog behavior remain matched to the existing desktop interface.
