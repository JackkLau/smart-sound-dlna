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

A Windows/macOS desktop control surface for sending a local music collection to a DLNA speaker. The current implementation uses a Rust backend and Tauri 2 native shell, with a React 19 renderer built by Vite. The visual reference is a silver perforated speaker beside a quiet music-library window: cool paper-white surfaces, charcoal labels, and one orange status light. This is a working single-screen tool, not a marketing page. The brief and agreed Tauri IPC contract are the authority. The established Python browser interface supplied the visual and interaction reference; its retained HTTP surface and the phone interface also use React, with their existing backend contracts and layout preserved.

The audience is Chinese-speaking people using a computer on the same local network as their speaker. The supported UI locale is `zh-CN`; the locale alone does not imply a regulated market. No Japanese-market or Japanese-data flows apply. The application does not claim Xiaomi affiliation or model compatibility that device discovery has not established.

The signature is the CSS speaker illustration with a restrained perforated grille and a connection light. Controls, playback information, and error recovery remain conventional. Avoid dashboard metric cards, marketing hero sections, fake album art, fake music/device content, and remote font/CDN dependencies.

Runtime token ownership is Model B: `frontend/style.css` is the canonical runtime owner for the desktop application; this document mirrors accepted values and explains their use. The retained browser surface keeps its established `xiaomi_dlna/static/style.css`; desktop changes are owned by `frontend/`. No framework theme adapter or additional styling system is introduced. When tokens change, update both files together. The product offers one light theme plus system forced-colors support.

## Colors

The 6 leading identity colors are background `#f5f6f8`, surface `#ffffff`, ink `#24272d`, muted `#626975`, accent `#ff6900`, and line `#dfe3e8`. The brighter Xiaomi-like orange is decorative only. Primary actions use the darker orange `#c74600` so small white labels retain contrast. Selection uses a warm tint plus pressed-state semantics and an icon. Success and error have text labels; color never carries status alone. Blue focus outlines remain distinct from orange selection. The decorative speaker's metallic gradient values are illustration-specific, not general interface tokens.

## Typography

Display headings use the locally available Avenir Next with Chinese-capable PingFang/Microsoft YaHei fallbacks, 27–32px for the library title and 18–20px for the selected speaker. Body controls use the OS Chinese sans stack, a 14px base, and generous Chinese line height. Secondary metadata is 10–12px, never the only label for an action. Numeric time, file size, and pagination use the local monospace stack for stable widths. Titles and filenames wrap; the player title may scroll within its reserved two-line region. No text relies on hover for full-value access.

## Layout

Desktop: a 280px device column beside a flexible music collection, with 20px between them and a fixed bottom player. The page max width is 1400px. A small laptop narrows the device column and lets import buttons wrap. Below 620px, the device section becomes a compact summary above the music library and playback controls use three rows. The document owns vertical scrolling; only dialog bodies and unusually long player names scroll internally. No viewport-height constraint is applied to the page shell.

The track list has a 340px minimum surface (320px in the narrow layout), 20 records per page, an explicit range and previous/next controls. On small screens the size column is omitted while title, format, and play action remain. The current search and page are restored through guarded sessionStorage in the native window. Desktop state is not shareable navigation, so this is an intentional exception to URL-state defaults. Storage failure leaves working in-memory state. The fixed player always has matching document bottom padding and scroll-padding; narrow screens include safe-area space.

## Elevation & Depth

Use subtle borders and tonal surfaces for library/device hierarchy. The playback dock, transient status toast, dialogs, and authored select popup receive elevation shadows. Controls additionally use a 1px edge highlight; text fields use a restrained inset shadow. Background blur is reserved for a native dialog backdrop. The CSS speaker has illustration-only shadows to describe its shape; do not reuse them on records or controls.

## Shapes

Controls use a 9px radius, device panels 18px, and dialogs 20px. The smaller library surface uses 12px as a documented content-frame variant. Play is a circular transport button. Other transport icons occupy real 34px buttons; dense mobile player controls are at least 24px. Avoid arbitrary pill badges and decorative nested cards.

### Library before import

The user prioritizes listening over importing: the library heading, search, track list and pagination precede the import panel in document and keyboard order. File/folder import, phone sending and Bilibili import share one section below the list, followed by their import feedback and download queue. The section uses existing button and field tokens, with four action columns on wide windows and two at 1150px and below. It remains in natural document flow with the existing fixed-player clearance. The empty-library first-song action remains beside its empty-state explanation.

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
| Table Selection | React `PartPicker` and `BilibiliSelection` | favorites import contract | per-part/all-parts; cross-page retention | selection and React import tests |
| Select/Listbox | shared React `Select` in `components.jsx`, backed by Radix Select | this contract and `frontend/style.css` | desktop authored; phone and retained Python surface intentionally keep native OS pickers | opened popup, keyboard, dialog portal and narrow layout |
| Form | React `Dialog`, feature `Form` / `ErrorMessage`, and component-owned validation with synchronous mutation guards | agreed Tauri IPC commands and this contract | device URL, imports, voice and song names; file/folder selection remains native | invalid, pending, success, error, focus and IME checks |
| Scrollbar | global `style.css` selectors | runtime tokens and this document | dialog geometry only | static audit and computed global style |
| Toast | React `useDesktop.announce` and `#toast`; retained browser `announce` state | this contract | success/info | polite live region, one fixed slot |
| CRUD | React `useDesktop` mutation, state acceptance, and import hooks | agreed IPC contract | import/select/control/rename/remove from library; audio files retained | native backend and desktop full-flow validation |

This single screen has no file deletion or separate detail workflows; a second UX-CONTRACT.md is intentionally unnecessary. The contract is collected here so owners remain unambiguous.

### Shared interaction contract

Clickable controls use native buttons, links, inputs and dialogs; authored select behavior is supplied by the maintained Radix primitive. Enabled buttons define hover, pressed, focus-visible, disabled and busy states. Busy requests preserve label dimensions and disable duplicate control submissions. Desktop select geometry is application-owned through Radix; phone and retained Python select popups remain OS-owned. Native modal dialogs own focus trapping, inert background, Escape and restoration. During a submitted device request, the authored dialog stays open until the native operation settles and cannot be dismissed. Before submission, cancellation preserves entered values for reopening. Native file and folder dialogs are explicitly OS-owned and are opened by Rust commands; choosing Cancel returns null and makes no import claim. This is not a browser alert/confirm/prompt substitution. Standard OS window chrome owns window controls; the renderer does not invent a draggable titlebar or emulate platform controls.

Forms use `novalidate`, linked labels/help/errors, first-error focus and preserved values. Search is local and immediate, composition-safe, clearable, and page-clamped. No remote search or debounce is needed. List records and device options are rendered only when their data changes, preserving active controls across background polling.

The Rust backend supplies every device, track and playback state through `window.__TAURI__.core.invoke`. No fetch, HTTP control token, browser file upload, JavaScript filesystem path access or external script is used. The Tauri shell enables `withGlobalTauri`; production assets are relative `./app.css` and `./app.js` within `dist/desktop/`, generated from JSX modules. Opening the page outside the desktop app displays a persistent instruction to use the client and leaves native actions disabled.

Initial, empty, no-results, disconnected, failed, transition and confirmed states have distinct copy. Native commands are never automatically replayed. IPC cannot be aborted: slow operations display status and keep mutation protection until the original invocation settles. `get_state` polls every two seconds without overlap, and generations reject results that began before a mutation. Focused seek/volume controls are not reset by polling. A successful action accepts returned backend state; subsequent polling reconciles cached speaker status.

The backend discovers devices once on startup; healthy or unselected devices are not periodically rediscovered. Current-device health polling remains active. `device_connection` is `unselected`, `checking`, `connected`, or `disconnected`; only confirmed connections enable playback. Disconnection keeps the selected device visible, offers immediate retry by clicking that device, and retries automatically with a 2/4/8/16/30-second capped backoff. Only while disconnected, rediscovery runs approximately every 30 seconds to refresh the same device ID/address. Recovery never starts playback or selects another device. User actions preempt read-only status queries without cancelling in-flight playback mutations. Optional position/volume reads time out after one second and cannot mark a responsive renderer offline. `discovering` and `discovery_error` drive honest busy/failure copy; the renderer does not infer discovery success from a timer. A manual rescan uses `discover_devices`. Only backend-provided `is_xiaomi` and `is_xiaomi_pro` create Xiaomi/Pro badges; Pro devices are presented first, followed by other Xiaomi and generic DLNA speakers. Device selection remains explicit.

`pick_music_files` and `pick_music_folder` open OS pickers. Import references local originals, displays a native-selection/waiting state and then confirmed `{added,total,skipped,warnings}`; cancellation hides pending feedback without claiming success. There is no browser upload queue, file-size transfer limit, cloud upload, or unverified drag-and-drop API. The formats and collection limit come from `get_config`. Import failures remain inline with a reselect/retry path. Playback is disabled until a speaker connection is confirmed, with an explicit nearby reason.

### Compact controls and dialog scroll ownership

The default density serves repeated library and import operations: 28px desktop page margins, a 72px top bar, 20px workspace gap, 56px minimum track rows, and 40px buttons/text inputs. Narrow layouts use 16px page margins and a 64px top bar. Text sizes, label hierarchy and the existing focus/disabled states remain unchanged. Form sections use 12px separation, with 6–8px gaps inside a group. Empty error regions keep a 20px reserved line to avoid shifting adjacent actions when feedback arrives.

Every authored dialog shares one frame: `.dialog-heading` is a non-scrolling sibling of `.dialog-body`. The open dialog uses a column flex layout bounded by the viewport; only the body has vertical overflow, a stable scrollbar gutter and contained overscroll. The title and close button remain visible at every body scroll position. Header and body use 20px horizontal padding (16px in the narrow layout); body controls have 12px scroll margins so keyboard focus remains clear of the frame edges. The manual-device form is itself the body; feature dialogs wrap their existing forms in the shared body. Primary actions remain part of their form and are reachable by body scrolling. Native modal focus, Escape, pending-dismissal protection, validation and draft behavior are owned by React state, the shared `Dialog` component and form hooks.

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

`TrackDialogs.jsx` reuses the shared React `Dialog`, scoped `useDesktop.mutate` / `announce`, and the fixed-heading native dialog frame for removal. The confirmation names the track, says original and downloaded audio files remain, explains that custom names must be set again after reimport, and warns that removing the current track stops playback. Cancel receives initial focus; waiting blocks dismissal and duplicates. Errors stay inline with the song still shown, and cancellation or success returns focus to the same row or the next row at that position, falling back to the library heading for an empty result.

Removal only persists an index change and rolls back the record at its original position on save failure. A current-track removal first stops the renderer; stop failure keeps the record. Successful current-track removal clears the current ID, URI and completion evidence without starting another song. Other removals send no playback requests. Removed IDs no longer receive new media requests. Existing search/page state is retained and clamped after removal, including the last item on a page. Explicit local reimport or a fresh Bilibili import can restore the music; completed download history does not substitute for current library membership.

### Song names and voice playback

Each library row retains its play control and adds the text action `曲名`. `TrackDialogs.jsx` uses the native React `Dialog`, component-owned draft/validation/pending state, and `useDesktop.mutate` / `announce` state and feedback owners. It does not introduce a second form or mutation framework. Its 520px dialog is a content-width variant with the existing spacing, color and field tokens. Song naming works without a connected speaker.

Recognition reads local audio title tags, then conservative filename or video-title hints; it neither uploads audio nor claims acoustic fingerprinting. New imports use a suggestion when available. Existing rows change only after Save. Recognition changes the draft and identifies its source; no reliable result keeps the draft. Manual saves persist across restart and reimport and become the voice matching title immediately. Saving does not restart playback or replace the audio file. Names must contain letters or numbers, have at most 120 Unicode characters and no control characters; names equal under voice normalization require a singer/version distinction.

The form keeps a live Chinese example `小爱同学，播放本地 + 曲名`, with a reminder to enable voice reception. Pending actions prevent duplicate submissions and dialog dismissal; failure stays inline with the input preserved and focused. IME composition never submits. Cancel preserves the per-song draft for reopening during this session. Closing returns focus to that song's current row action or the library heading if filtering removed it. Polling cannot overwrite an open draft. An expected-title conflict asks the user to reopen the editor instead of silently overwriting newer data.

All controls remain Chinese-localized on Windows and macOS. Native chooser geometry, system title bar and select popup are OS-owned; button/form/status behavior and theme tokens remain application-owned. React event handlers own the UI; no global UI controller or imperative element-construction path remains.

### Iconography and motion

The shared React `Icon` component’s locally defined 24px SVG paths use consistent 1.7px rounded strokes. Icon-only controls have Chinese names and native title hints. Text labels remain on all import and setup actions. Motion is limited to busy discovery/transition signals and 150ms state feedback. Reduced-motion preferences suppress animations and transitions.

### Content voice

Use brief practical Chinese: 添加音乐, 导入文件夹, 重新搜索音响, 继续播放. Avoid marketing claims, protocol trivia in routine controls, arbitrary demo songs, or promises that every Xiaomi speaker supports DLNA. Setup copy explains the supported-DLNA requirement and same-network requirement. Desktop lifecycle copy explicitly says the client must remain running during playback. Desktop playback never requires a separate browser or Python service. The explicitly enabled phone-import companion uses the phone browser, as defined below.

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

### Phone import on a trusted LAN

The user requested same-Wi-Fi Android-to-PC link submission. React `MobilePanel` reuses the canonical feature Form, error, native Dialog, fixed heading and button recipes. “手机发送” explicitly opens an isolated receiver; closing its dialog does not revoke access, while “暂停手机接收” stops requests but preserves the phone credential. “忘记手机并重新配对” explicitly revokes it and creates a new QR. Receiver startup is never automatic. The desktop displays a locally generated QR, never a URL to type or numeric code. Multiple interfaces use the canonical native select with IP labels. The fixed port is 8998; a conflict is reported instead of changing the bookmark address. “已记住手机” describes a persisted pairing, never live connectivity. Reopening the dialog refreshes state without overwriting a pending action.

The new `mobile/` surface is the intentional phone-browser exception to the desktop-only interface: no Tauri bridge or account form; paired playback controls are added under the explicit 0.8.0 request. `frontend/style.css` stays the canonical token/control/scrollbar owner; `mobile/mobile.css` adds only single-column layout, 44px touch targets and phone geometry. It accepts manually pasted share text without requesting clipboard access. Native checkboxes choose individual parts; only the linked P is initially selected. Busy requests preserve drafts and prevent duplicates. Error text remains inline, provider errors explain that login occurs on PC, and transient connection failure retains the selected video for retry. The first scan redeems a one-use random ticket automatically and immediately removes the fragment from browser history. The resulting bearer token is remembered in localStorage, as explicitly requested for bookmarks/home-screen shortcuts. On opening, jobs authentication must succeed before showing connected. Transient errors retain the token/draft; 401 clears it. Storage failure falls back to memory with a visible explanation that reconnection will need pairing.

The LAN boundary exposes pairing, parsing, selected-part enqueue, sanitized status for this session's jobs, and the paired library/playback operations described below. One-use random QR tickets expire in ten minutes with five guesses; they are in URL fragments only, not query strings. Long-term bearer tokens stay out of URLs, Origin/Host checks reject cross-origin or DNS-rebinding requests, and only previews created by that phone session are accepted. Pausing the receiver drops in-flight requests; previously queued downloads continue under the desktop engine. Restart/re-enable restores the SHA-256 credential digest from the app data directory, never plaintext bearer material. Forgetting first stops the server and then removes its saved digest; a failed deletion is reported and leaves reception paused. Server shutdown is bounded. QR generation uses the local MIT qrcode-generator 1.4.4 dependency with a four-module quiet zone; QR black/white is an optical requirement, not a theme token. No provider cookies, file paths, media URLs or arbitrary command execution are exposed. The explicit 0.8.0 request extends the same pairing boundary with paged song titles and allowlisted playback controls. HTTP is limited to a user-enabled trusted-network flow; no public relay or Android share-target APK is claimed. This narrow opt-in import endpoint extends the previous LAN boundary without adding a general control API.

The phone's text fields expose a localized clear action when non-empty. Search clearing immediately refreshes the unfiltered first page and restores focus. Clearing video/share text also removes its unsubmitted preview, selection and related validation feedback, then focuses the textarea; queued downloads continue. Foreground requests disable clearing until they settle. The textarea clear target is at least 44px and has a reserved inset so it never covers the input text or changes the field footprint.

### Persistent account sessions (0.6.0)

The user explicitly requested login restoration across restarts and upgrades. `SessionCache` owns provider-scoped macOS Keychain / Windows Credential Store access; its stable service identifier has no app-version or install-path component. It never falls back to plaintext or keyring's mock backend. Only micoapi `userId` / `serviceToken` and allowlisted Bilibili cookies are stored; passwords, Xiaomi account `passToken`, signed URLs, the library index and frontend storage remain excluded. The per-provider `LoginFlow` gate serializes restore, login and logout; a late cancelled save is cleared. Logout first removes the stored credential and reports an error if removal fails, so it never falsely promises a persistent logout.

Startup restores both accounts in background tasks without blocking local music. A saved credential is not a connected account: service validation must succeed before publishing connected state. Failed validation keeps retry data and shows a persistent inline error; expiry still requires login. React `Account.jsx` remains the single owner for both providers' restoring, saved-but-offline, connected and cleared states. “退出登录” clears that provider only. Voice binding and listening remain explicit, session-local actions and are never silently enabled by account restoration. Validation includes memory-boundary failure/race checks, real OS credential read/write/delete across processes, and shared renderer tests. No visual tokens or dialog/scroll owners change.

Xiaomi account credentials (`passToken`) are sent only to the fixed Xiaomi account endpoint. The STS request is restricted to the exact HTTPS MiNA host and contains the signed challenge, never the account Cookie. Empty authorization failures surface in the main dialog instead of leaving an unexplained waiting state. All challenge material stays in memory.

### Favorites import (0.4.0)

This user-requested extension intentionally replaces the one-part-only import rule. Video and favorites sources share `BilibiliSelection` in `frontend/import-selection.js` and the React `PartPicker` in `frontend/src/features/BilibiliPanel.jsx`. Native radio inputs own source switching; native checkboxes own individual and per-video all-selection; native select owns the account folder picker. The shared React form, error, notification, button, field and dialog owners remain canonical. No visual tokens change.

Favorites use server pagination of 20 videos, explicit previous/next buttons, total/page status, and unavailable/empty/error states. Selections survive paging and switching sources within this dialog session; a successfully opened different folder clears its previous selection. Metadata is bounded and refreshed explicitly. Both sources submit at most 100 selected CIDs; accepted tracks are cleared, skipped tracks retain selection and have persistent per-track reasons. Clearing selection does not delete music. Foreground import requests lock the source and selection until completion, with no silent replay. Download workers remain sequential and keep up to 200 recent job records. Closing the dialog retains view state and does not stop downloads.

| Capability | Canonical owner | Source of truth | Allowed variants | Verification |
| --- | --- | --- | --- | --- |
| Table Selection | `BilibiliSelection` and shared audio picker | this favorites contract | per-part, all parts of one video; cross-page retention | `tests/import-selection.test.cjs`, native app acceptance |

Reconcile drift: the prior single-part / no-playlist behavior changes under the explicit favorites request; token values, typography, native select ownership, notifications and dialog behavior remain matched to the existing desktop interface.

### Phone playback and queue clarity (0.8.0)

The explicit user request extends the remembered phone pairing to song selection and playback modes. The existing phone `api`, `action`, IME-safe `form`, connection restoration and polling owners serve import and playback together. Native `select#phone-mode` uses the same three modes as desktop; a deliberate Apply action protects an unsubmitted selection from background polling. There is no new modal or toast owner. Canonical desktop controls and tokens remain unchanged.

The desktop adds “播放全部” beside search, which atomically starts the first library track and chooses sequential mode. A stable help line above pagination names the next track, or explains the final boundary. Clicking any row starts from that row in the full PC library; filtering and pagination never alter queue order. Switching mode changes continuation without restarting the current song. Import remains beneath the library.

The phone places PC library and player above video import. Search is server paged at 20 records, debounced 300ms and composition safe; Clear is immediate and returns focus. Query and page remain memory-only to avoid placing private song searches into bookmarked connection URLs. Empty, no-results, loading, error, disconnected and current-song states have text. One native button owns each playable row. Previous/next page, explicit refresh and 44px transport buttons are reachable in natural document scroll, with no fixed phone dock. Background reads preserve focus and mode drafts. Mutations invalidate older reads and are never replayed automatically; a temporary failure disables stale playback controls while retaining pairing/import drafts.

`GET /api/library` returns only bounded IDs, titles, formats, counts and a sanitized player; `POST /api/playback` accepts typed play/play-all/control/mode actions. It cannot select a speaker, change accounts or pass paths/URLs. Track playback and transport commands include the speaker ID last seen by the phone; the engine checks current selection and connection under its existing action lock before sending SOAP. Phone, desktop and voice share queue state and serialization. Revocation, Host/Origin validation, shutdown and bearer checks apply equally to the added routes.

Queue completion combines a confirmed PLAYING-to-ended transition with renderer time or bounded local audio-property duration. A monotonic observation can cover missing/zero progress but never interrupts PLAYING by timer. A fresh valid near-end-to-head progress wrap also completes a track when Xiaomi firmware remains PLAYING on the same URI; missing position and elapsed time alone never trigger that path. The sequential last-track boundary explicitly stops a still-playing renderer. Pause, explicit stop, seek, outage and takeover invalidate prior observations. Supported renderers receive NORMAL so their retained repeat mode does not compete with the PC queue; unsupported optional mode operations retain compatibility. Verification: engine completion/property tests, wire regression in native CI, phone API boundary/projection checks, real controller UI tests and browser fixtures at narrow widths.


### React migration and verification

All three UI surfaces render declarative JSX with React state and effects. Shared `frontend/src/components.jsx` owns icons and native dialogs; `utils.js` owns formatting. `useDesktop.js` owns IPC polling and mutation generations, while `features/useFeatureController.js` owns feature polling, request locks and account transitions. The phone and retained Python browser keep separate transport hooks because their authentication and API contracts differ. Imperative DOM access is limited to mounting React and platform focus/dialog APIs; do not reintroduce `createElement`, `innerHTML`, global controllers or direct event wiring to construct product UI. QR generation is a pure local encoder; React owns the image.

Vite produces one local JS bundle and one CSS file per surface. Desktop production CSP stays strict; only development CSP permits Vite’s local refresh runtime. Phone and Python HTTP boundaries, Host/Origin and bearer/control-token checks are preserved. Built outputs are ignored and must be generated before Rust/Python startup. CI builds all UI assets before native tests and packaging.

| Previous rule / implementation | React outcome | Verdict | Evidence |
| --- | --- | --- | --- |
| Palette, spacing, native selects, speaker illustration | Existing CSS and token values reused | MATCH | production browser screenshots and strict audit |
| Imperative renderer and global feature handlers | Component-owned JSX, hooks and shared Dialog/Form | APPROVED MIGRATION | explicit React request; component tests |
| Native IPC and phone/Python HTTP boundaries | Commands, argument shapes and authorization retained | MATCH | Rust/Python integration and React boundary tests |
| Drafts, focus and pending dismissal | Native modal + React state; post-request focus after enabling controls | MATCH | editor, account, phone and upload regression tests |
| Focused slider polling | Preserve draft during interaction, reconcile backend value on blur/settle | MATCH | focused range regression tests |

`tests/react/` exercises rendered components and negative/race paths rather than fake DOM implementations. `scripts/smoke-ui.mjs` checks built resources under production CSP, desktop control/edit/search, native dialog focus and Escape, keyboard modes, narrow layouts, phone playback/import and retained-browser rendering. Backend tests exercise actual local HTTP/SOAP. Browser fixtures do not establish real speaker or cloud-account compatibility.


### Refined controls and video platforms (0.9.0)

The explicit request for more tactile controls updates the desktop canonical Select owner to `components.jsx` / Radix Select. The existing palette, compact density, library-first hierarchy and fixed dialog headings stay authoritative. `frontend/style.css` adds `--control-shadow` (`0 1px 2px #24272d0a, inset 0 1px 0 #ffffffcc`), `--field-shadow` (`inset 0 1px 2px #24272d08`) and `--popup-shadow` (`0 12px 32px #24272d24, 0 2px 6px #24272d0d`). Buttons/selects have a satin edge; fields are inset until focused. These shared rules also style the phone's fields. Phone and retained Python selects intentionally retain platform pickers; `premium-ui.json`'s native ownership entry records these exceptions, while the map above records the desktop authored owner.

Desktop playback mode, voice speaker, favorite folder and network-interface selectors share one wrapper, including selected checks, keyboard navigation, typeahead, Escape, disabled state and focus restoration. Popup width follows the trigger, wraps long labels, uses the global scrollbar, and flips at viewport edges. A native dialog provides the popup portal container so its menu is in the same top layer and remains interactive. Native range inputs retain keyboard/pointer semantics and existing draft/commit protection, with a thin filled track, white outlined thumb and visible numeric volume. Reduced motion and forced-colors remain supported.

“从视频导入” opens the existing dialog. An explicit radio group chooses 哔哩哔哩 or 抖音. Bilibili retains single/multi-P, favorites and account flows. Douyin accepts one public video URL or share message, previews title/author/duration and offers its video soundtrack. It does not import slideshows, livestreams or a separate background-music catalog. The shared clearable field applies to video and favorites links; clearing resets the unsubmitted preview/selection/errors and returns focus. Editing a video link or changing platform invalidates the old video preview. Foreground requests lock these controls, and queued downloads continue independently.

`parse_video({platform,url})` selects the native provider; legacy Bilibili commands remain compatible. Public Douyin SSR is tried first. If its metadata is unavailable, a short-lived incognito official webview loads the video and requests its public detail from that origin. It exports only a validated matching video ID, title, author, duration and allowlisted media URLs through a nonce-scoped intercepted callback; no account cookies or Tauri capability are granted to remote content. Manual close cancels; a 120-second deadline closes an unfinished attempt. Challenges are not solved automatically. Signed media URLs stay in native memory and are omitted from previews/index/logs.

Both providers use the same bounded queue, cancel/retry controls, atomic publication and library deduplication. Douyin temporarily downloads MP4, copies only AAC samples into M4A, and retains the original sample description including HE-AAC SBR/PS configuration. No transcoding, external executable or permanent video copy is introduced. Progress distinguishes downloading, extracting and importing. Original title recognition, manual rename and voice naming work for the resulting track as for other library imports. Phone import remains Bilibili in this iteration; phone playback and mode synchronization are unchanged.
