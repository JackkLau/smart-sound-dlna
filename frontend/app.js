'use strict';

(() => {
  const $ = (id) => document.getElementById(id);
  const PAGE_SIZE = 20;
  const localeNumber = new Intl.NumberFormat('zh-CN');
  const invoke = window.__TAURI__?.core?.invoke;
  const desktopAvailable = typeof invoke === 'function';
  let query = '';
  let page = 1;
  try {
    const saved = JSON.parse(sessionStorage.getItem('music-library-view') || '{}');
    query = typeof saved.query === 'string' ? saved.query : '';
    page = Math.max(1, Number.parseInt(saved.page, 10) || 1);
  } catch { /* Session restoration is optional; blocked storage must not block playback. */ }
  let composing = false;
  let deviceFormComposing = false;
  let config = null;
  let state = {
    tracks: [], devices: [], selected_device_id: null, device_connection: 'unselected', current_track_id: null,
    transport_state: 'STOPPED', position: 0, duration: 0, volume: null,
    status_error: null, warnings: [], queue_mode: 'sequential', discovering: false,
    discovery_error: null, interfaces: [], media_port: 0,
  };
  let loaded = false;
  let connected = false;
  let connectionError = '';
  let connecting = false;
  let polling = false;
  let mutationBusy = false;
  let discoveryPending = false;
  let importPending = false;
  let generation = 0;
  let toastTimer = null;
  let actionError = '';
  let slowMessage = '';
  let trackSignature = '';
  let deviceSignature = '';

  function icon(name, extraClass = '') {
    const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
    svg.setAttribute('class', `icon ${extraClass}`.trim());
    svg.setAttribute('aria-hidden', 'true');
    const use = document.createElementNS('http://www.w3.org/2000/svg', 'use');
    use.setAttribute('href', `#i-${name}`);
    svg.append(use);
    return svg;
  }
  function element(tag, className, text) {
    const node = document.createElement(tag);
    if (className) node.className = className;
    if (text !== undefined) node.textContent = text;
    return node;
  }
  function messageOf(error) {
    if (typeof error === 'string' && error.trim()) return error;
    if (error && typeof error.message === 'string') return error.message;
    return '操作未完成，请重试；如果问题持续，请重新启动客户端。';
  }
  function announce(message) {
    clearTimeout(toastTimer);
    $('toast').textContent = message;
    $('toast').hidden = false;
    toastTimer = setTimeout(() => { $('toast').hidden = true; }, 5500);
  }
  $('toast').addEventListener('mouseenter', () => clearTimeout(toastTimer));
  $('toast').addEventListener('mouseleave', () => {
    toastTimer = setTimeout(() => { $('toast').hidden = true; }, 3500);
  });
  function formatSize(bytes) {
    const value = Math.max(0, Number(bytes) || 0);
    return value < 1048576 ? `${(value / 1024).toFixed(0)} KB` : `${(value / 1048576).toFixed(1)} MB`;
  }
  function formatTime(seconds) {
    const value = Math.max(0, Math.floor(Number(seconds) || 0));
    const hours = Math.floor(value / 3600);
    return hours
      ? `${hours}:${String(Math.floor(value / 60) % 60).padStart(2, '0')}:${String(value % 60).padStart(2, '0')}`
      : `${Math.floor(value / 60)}:${String(value % 60).padStart(2, '0')}`;
  }
  function rememberSearch() {
    try { sessionStorage.setItem('music-library-view', JSON.stringify({ query, page })); }
    catch { /* In-memory view state still works if session storage is unavailable. */ }
  }
  function setConnected(value, message = '') {
    connected = value;
    connectionError = value ? '' : message || '暂时无法读取客户端状态。';
    $('connection-status').className = `connection ${value ? 'online' : 'offline'}`;
    const label = !desktopAvailable ? '需要桌面客户端' : value ? '客户端已就绪' : '客户端未就绪';
    $('connection-status').replaceChildren(element('span', 'status-dot'), document.createTextNode(label));
    $('connection-error').hidden = value;
    $('connection-error-text').textContent = message || '暂时无法读取客户端状态。请重新检查，或重新启动客户端。';
    $('retry-connection').hidden = !desktopAvailable;
    if (!loaded) renderLibrary();
    renderControls();
  }
  async function loadConfig() {
    config = await invoke('get_config');
    const extensions = Array.isArray(config.formats) ? config.formats : [];
    $('formats').textContent = `${extensions.map((extension) => extension.toUpperCase()).join(' · ')}${config.max_tracks ? ` · 最多 ${localeNumber.format(config.max_tracks)} 首` : ''}`;
  }
  async function refresh() {
    if (!desktopAvailable || polling || mutationBusy) return;
    polling = true;
    const currentGeneration = generation;
    // IPC cannot be cancelled. Keep this poll locked until it actually settles,
    // so a slow native operation never causes overlapping get_state commands.
    const slowTimer = setTimeout(() => {
      if (currentGeneration === generation) setConnected(false, '客户端响应较慢，正在等待当前操作完成。');
    }, 12000);
    try {
      const data = await invoke('get_state');
      if (currentGeneration === generation) {
        acceptState(data);
        setConnected(true);
      }
    } catch (error) {
      if (currentGeneration === generation) {
        setConnected(false, messageOf(error));
        if (!loaded) renderLibrary();
      }
    } finally {
      clearTimeout(slowTimer);
      polling = false;
    }
  }
  async function mutate(command, args = {}, successMessage = '', { scopedError = false } = {}) {
    if (mutationBusy || !config || !connected) return null;
    mutationBusy = true;
    generation += 1;
    actionError = '';
    slowMessage = '';
    renderControls();
    renderErrors();
    const slowTimer = setTimeout(() => {
      slowMessage = '操作仍在进行，请稍候。为避免重复执行，完成前无法再次操作。';
      renderErrors();
    }, 15000);
    try {
      const data = await invoke(command, args);
      if (data && Array.isArray(data.tracks)) acceptState(data);
      if (successMessage && data !== null) announce(successMessage);
      return data;
    } catch (error) {
      if (!scopedError) { actionError = messageOf(error); renderErrors(); }
      throw error;
    } finally {
      clearTimeout(slowTimer);
      slowMessage = '';
      mutationBusy = false;
      renderErrors();
      renderControls();
    }
  }
  function acceptState(data) {
    state = {
      ...state, ...data,
      tracks: Array.isArray(data.tracks) ? data.tracks : [],
      devices: Array.isArray(data.devices) ? data.devices : [],
    };
    loaded = true;
    renderLibrary();
    renderDevices();
    renderControls();
    renderErrors();
  }
  function renderErrors() {
    const messages = [actionError, slowMessage, state.status_error, ...(Array.isArray(state.warnings) ? state.warnings : [])].filter(Boolean);
    $('playback-error').hidden = !messages.length;
    $('playback-error-text').textContent = [...new Set(messages)].join(' ');
    $('discovery-error').hidden = !state.discovery_error;
    $('discovery-error').textContent = state.discovery_error || '';
  }
  function renderLibrary() {
    const normalizedQuery = query.toLocaleLowerCase('zh-CN').trim();
    const tracks = state.tracks.filter((track) => `${track.title || ''} ${track.filename || ''}`.toLocaleLowerCase('zh-CN').includes(normalizedQuery));
    const pages = Math.max(1, Math.ceil(tracks.length / PAGE_SIZE));
    if (loaded) { page = Math.min(Math.max(1, page), pages); rememberSearch(); }
    $('track-count').textContent = localeNumber.format(state.tracks.length);
    $('clear-search').hidden = !query;
    $('page-info').textContent = loaded ? `${page} / ${pages}` : '1 / 1';
    $('page-previous').disabled = !loaded || page <= 1;
    $('page-next').disabled = !loaded || page >= pages;
    $('list-status').textContent = !loaded
      ? !desktopAvailable ? '音乐库需要桌面客户端' : connectionError ? '音乐库暂时不可用' : '正在读取音乐库…'
      : tracks.length ? `第 ${localeNumber.format((page - 1) * PAGE_SIZE + 1)}–${localeNumber.format(Math.min(page * PAGE_SIZE, tracks.length))} 首，共 ${localeNumber.format(tracks.length)} 首`
      : query ? '没有匹配的音乐' : '还没有添加音乐';
    $('empty-state').hidden = tracks.length > 0;
    if (!tracks.length) {
      $('empty-title').textContent = !desktopAvailable ? '请在桌面客户端中打开'
        : !loaded ? connectionError ? '暂时无法读取音乐库' : '正在读取音乐库' : query ? '没有找到这首歌' : '这里，留给你的好音乐';
      $('empty-copy').textContent = !desktopAvailable ? '此界面需要小米音响桌面客户端，无法在普通浏览器中控制音响。'
        : !loaded ? connectionError ? '请点击上方「重新检查」。如果仍未恢复，请重新启动客户端。' : '客户端正在准备本地音乐和播放设备。'
        : query ? `试试其他歌名或文件名，或清空「${query}」。`
        : '添加本地音乐，或导入整个文件夹。连接音响后，就能把音乐送到耳边。';
      $('empty-add').querySelector('span').textContent = query ? '清空搜索' : '添加第一首音乐';
      $('empty-add').disabled = !query && (!connected || !config || mutationBusy || importPending);
      $('formats').hidden = Boolean(query) || !loaded;
    }
    const slice = tracks.slice((page - 1) * PAGE_SIZE, page * PAGE_SIZE);
    const signature = JSON.stringify([slice, state.current_track_id, page]);
    if (signature !== trackSignature) {
      trackSignature = signature;
      const focusTrack = document.activeElement?.dataset?.trackId;
      const focusAction = document.activeElement?.dataset?.trackAction;
      const rows = slice.map((track, index) => {
        const current = track.id === state.current_track_id;
        const row = element('li', `track-row${current ? ' current' : ''}`);
        if (current) row.setAttribute('aria-current', 'true');
        const main = element('div', 'track-main');
        const number = element('span', 'track-number', current ? '♪' : String((page - 1) * PAGE_SIZE + index + 1).padStart(2, '0'));
        const text = element('div', 'track-text');
        text.append(element('strong', 'track-title', track.title || track.filename));
        text.append(element('span', 'track-filename', track.filename));
        main.append(number, text);
        const play = element('button', 'icon-button track-play');
        play.type = 'button';
        play.dataset.trackId = track.id;
        play.dataset.trackAction = 'play';
        play.setAttribute('aria-label', `播放 ${track.title || track.filename}`);
        play.title = `播放 ${track.title || track.filename}`;
        play.append(icon('play'));
        play.addEventListener('click', () => playTrack(track.id));
        const edit = element('button', 'text-button track-edit', '曲名');
        edit.type = 'button'; edit.dataset.trackId = track.id; edit.dataset.trackAction = 'edit';
        edit.setAttribute('aria-label', `识别或修改曲名：${track.title || track.filename}`);
        edit.title = '识别或修改曲名';
        edit.addEventListener('click', () => window.MusicTrackEditor?.open(track, edit));
        const remove = element('button', 'text-button track-remove', '移除');
        remove.type = 'button'; remove.dataset.trackId = track.id; remove.dataset.trackAction = 'remove';
        remove.setAttribute('aria-label', `移出曲库：${track.title || track.filename}`);
        remove.title = '移出曲库，保留音频文件';
        remove.addEventListener('click', () => window.MusicLibraryActions?.openRemove(track, index));
        const actions = element('div', 'track-actions'); actions.append(edit, remove);
        // A native button fills the row; secondary controls remain siblings.
        row.append(play, main, element('span', 'track-format', (track.format || '').toUpperCase()), element('span', 'track-size', formatSize(track.size)), actions);
        return row;
      });
      $('tracks').replaceChildren(...rows);
      if (focusTrack) [...$('tracks').querySelectorAll('button')].find((button) => button.dataset.trackId === focusTrack && button.dataset.trackAction === focusAction)?.focus({ preventScroll: true });
    }
    renderTrackControls();
  }
  function renderTrackControls() {
    const reason = speakerUnavailableReason();
    const disabled = !connected || !config || Boolean(reason) || mutationBusy || importPending;
    document.querySelectorAll('.track-play').forEach((button) => {
      button.disabled = disabled;
      button.title = reason || button.getAttribute('aria-label');
    });
    document.querySelectorAll('.track-edit, .track-remove').forEach((button) => {
      button.disabled = !connected || !config || mutationBusy || importPending;
    });
    $('library-caption').textContent = state.tracks.length && reason ? reason : '本地音乐 · 直接推流';
  }
  function speakerUnavailableReason() {
    if (!state.selected_device_id) return '先连接音响，再播放音乐';
    if (state.device_connection === 'disconnected') return '音响已断开，正在自动重连';
    if (state.device_connection !== 'connected') return '正在确认音响连接…';
    return '';
  }
  function renderDiscovery() {
    const discovering = Boolean(state.discovering || discoveryPending);
    $('discover').setAttribute('aria-busy', String(discovering));
    $('discover').querySelector('span').textContent = discovering ? '正在搜索音响' : state.device_connection === 'connected' ? '搜索其他音响' : '重新搜索音响';
    if (!desktopAvailable) $('device-hint').textContent = '通过桌面客户端自动搜索音响';
    else if (connectionError) $('device-hint').textContent = '客户端未就绪，暂时无法搜索';
    else if (state.device_connection === 'disconnected') $('device-hint').textContent = '连接已断开，正在自动重连；点击当前音响可立即重试';
    else if (discovering) $('device-hint').textContent = discoveryPending ? '正在重新搜索局域网音响…' : '正在自动发现局域网音响…';
    else if (state.device_connection === 'checking') $('device-hint').textContent = '正在确认音响连接…';
    else if (state.device_connection === 'connected') $('device-hint').textContent = '已连接 · 需要更换设备时手动搜索';
    else if (state.discovery_error) $('device-hint').textContent = '搜索未完成，可重新搜索或手动添加';
    else if (state.devices.length) {
      const proCount = state.devices.filter((device) => device.is_xiaomi_pro).length;
      $('device-hint').textContent = proCount ? `发现 ${state.devices.length} 台音响，含 ${proCount} 台小爱音箱 Pro` : `发现 ${state.devices.length} 台音响，点击即可连接`;
    } else $('device-hint').textContent = loaded ? '暂未发现音响，可重新搜索或手动添加' : '启动后自动发现局域网音响';
  }
  function renderDevices() {
    const device = state.devices.find((item) => item.id === state.selected_device_id);
    $('device-title').textContent = device?.name || '连接你的音响';
    $('device-subtitle').textContent = device
      ? [state.device_connection === 'disconnected' ? '已断开 · 自动重连中' : state.device_connection === 'checking' ? '正在连接' : '已连接', device.model, device.address].filter(Boolean).join(' · ')
      : '让喜欢的音乐，回到房间里。';
    document.querySelector('.speaker-art').classList.toggle('connected', Boolean(device) && state.device_connection === 'connected');
    renderDiscovery();
    const devices = [...state.devices].sort((a, b) => Number(Boolean(b.is_xiaomi_pro)) - Number(Boolean(a.is_xiaomi_pro)) || Number(Boolean(b.is_xiaomi)) - Number(Boolean(a.is_xiaomi)));
    const signature = JSON.stringify([devices, state.selected_device_id, state.device_connection]);
    if (signature === deviceSignature) return;
    deviceSignature = signature;
    const focusDevice = document.activeElement?.dataset?.deviceId;
    const buttons = devices.map((item) => {
      const button = element('button', 'device-option');
      button.type = 'button';
      button.dataset.deviceId = item.id;
      button.setAttribute('aria-pressed', String(item.id === state.selected_device_id));
      const selected = item.id === state.selected_device_id;
      const statusLabel = state.device_connection === 'disconnected' ? '连接已断开，点击重试' : state.device_connection === 'checking' ? '正在连接' : '已连接';
      button.setAttribute('aria-label', `${selected ? statusLabel : '选择'} ${item.name}${item.is_xiaomi_pro ? '，小爱音箱 Pro' : ''}`);
      const label = element('span', 'device-label');
      const name = element('span', 'device-name-line');
      name.append(element('strong', '', item.name));
      if (item.is_xiaomi_pro) name.append(element('span', 'device-badge', '小爱 Pro'));
      else if (item.is_xiaomi) name.append(element('span', 'device-badge', '小米'));
      label.append(name, element('small', '', selected ? statusLabel : item.model || item.address || 'DLNA 音响'));
      button.append(icon('speaker'), label);
      if (selected && state.device_connection === 'connected') button.append(icon('check', 'small check'));
      button.addEventListener('click', async () => {
        if (item.id === state.selected_device_id && state.device_connection !== 'disconnected') return;
        try { await mutate('select_device', { deviceId: item.id }, `正在连接 ${item.name}`); }
        catch { /* Persistent recovery message is already shown. */ }
      });
      return button;
    });
    $('devices').replaceChildren(...buttons);
    if (focusDevice) [...$('devices').querySelectorAll('button')].find((button) => button.dataset.deviceId === focusDevice)?.focus({ preventScroll: true });
  }
  function renderControls() {
    const available = Boolean(connected && config);
    const speakerReason = speakerUnavailableReason();
    const ready = available && !speakerReason && !mutationBusy && !importPending;
    const current = state.tracks.find((track) => track.id === state.current_track_id);
    const device = state.devices.find((item) => item.id === state.selected_device_id);
    const playing = Boolean(current) && state.transport_state === 'PLAYING';
    const paused = Boolean(current) && state.transport_state === 'PAUSED_PLAYBACK';
    const externalPlayback = !current && ['PLAYING', 'PAUSED_PLAYBACK'].includes(state.transport_state);
    const transitioning = state.transport_state === 'TRANSITIONING';
    const transportLabels = { PLAYING: '正在播放', PAUSED_PLAYBACK: '已暂停', STOPPED: '已停止', NO_MEDIA_PRESENT: '已停止', TRANSITIONING: '正在准备播放…' };
    $('now-title').textContent = current ? current.title || current.filename
      : externalPlayback ? state.transport_state === 'PLAYING' ? '音响正在播放其他内容' : '音响已暂停其他内容' : '还没有播放音乐';
    $('now-subtitle').textContent = state.selected_device_id && speakerReason ? speakerReason : current
      ? `${transportLabels[state.transport_state] || '准备播放'}${device ? ` · ${device.name}` : ''}`
      : externalPlayback ? '选择本地音乐，即可切换播放'
      : device ? `播放到 ${device.name}` : '连接音响，选一首喜欢的歌';
    document.title = current ? `${playing ? '正在播放 · ' : ''}${current.title || current.filename} | 小米音响` : '小米音响 · 本地音乐';
    document.querySelector('.player').classList.toggle('is-playing', playing);
    $('play-icon').setAttribute('href', playing ? '#i-pause' : '#i-play');
    const playLabel = playing ? '暂停' : paused ? '继续播放' : '播放';
    $('play-pause').setAttribute('aria-label', playLabel);
    $('play-pause').title = speakerReason || playLabel;
    $('play-pause').disabled = !ready || !state.tracks.length || transitioning;
    $('play-pause').setAttribute('aria-busy', String(transitioning || mutationBusy));
    $('stop').disabled = !ready || !current || state.transport_state === 'STOPPED';
    $('previous').disabled = !ready || !state.tracks.length;
    $('next').disabled = !ready || !state.tracks.length;
    $('seek').disabled = !ready || !current || !state.duration || state.transport_state === 'STOPPED';
    if (document.activeElement !== $('seek')) {
      $('seek').max = String(Math.max(1, state.duration || 0));
      $('seek').value = String(Math.min(state.position || 0, state.duration || 0));
      $('elapsed').textContent = formatTime(state.position);
    }
    $('seek').setAttribute('aria-valuetext', `${formatTime($('seek').value)}，共 ${formatTime(state.duration)}`);
    $('duration').textContent = formatTime(state.duration);
    $('volume').disabled = !ready || state.volume === null;
    if (document.activeElement !== $('volume')) {
      if (state.volume !== null) $('volume').value = String(state.volume);
      $('volume-label').textContent = state.volume === null ? '—' : String(state.volume);
    }
    $('volume').title = state.volume === null ? '音响暂未提供音量控制' : '音量';
    $('mode').disabled = !available || mutationBusy || importPending;
    if (document.activeElement !== $('mode')) $('mode').value = state.queue_mode || 'sequential';
    $('discover').disabled = !available || mutationBusy || importPending || state.discovering || discoveryPending;
    ['open-folder', 'open-device', 'add-music', 'open-bilibili', 'open-voice'].forEach((id) => { $(id).disabled = !available || mutationBusy || importPending; });
    document.querySelectorAll('.device-option').forEach((button) => { button.disabled = !available || mutationBusy || importPending; });
    $('empty-add').disabled = !query && (!available || mutationBusy || importPending);
    renderDiscovery();
    renderTrackControls();
  }
  async function playTrack(trackId) {
    try { await mutate('play_track', { trackId }); }
    catch { /* Persistent error already shown. */ }
  }
  async function control(action, value) {
    try { await mutate('control', { action, value: value === undefined ? null : value }); }
    catch { /* Reconcile with the native state instead of claiming success. */ }
    await refresh();
  }
  $('play-pause').addEventListener('click', () => {
    const hasCurrent = state.tracks.some((track) => track.id === state.current_track_id);
    if (hasCurrent && state.transport_state === 'PLAYING') control('pause');
    else if (hasCurrent && state.transport_state === 'PAUSED_PLAYBACK') control('resume');
    else if (state.tracks.length) playTrack(hasCurrent ? state.current_track_id : state.tracks[0].id);
  });
  ['previous', 'next', 'stop'].forEach((id) => $(id).addEventListener('click', () => control(id)));
  $('seek').addEventListener('input', () => {
    $('elapsed').textContent = formatTime($('seek').value);
    $('seek').setAttribute('aria-valuetext', `${formatTime($('seek').value)}，共 ${formatTime(state.duration)}`);
  });
  $('seek').addEventListener('change', () => control('seek', Number($('seek').value)));
  $('volume').addEventListener('input', () => { $('volume-label').textContent = $('volume').value; });
  $('volume').addEventListener('change', () => control('volume', Number($('volume').value)));
  $('mode').addEventListener('change', () => control('mode', $('mode').value));
  $('discover').addEventListener('click', async () => {
    if (discoveryPending || state.discovering) return;
    discoveryPending = true;
    renderControls();
    try {
      const result = await mutate('discover_devices');
      if (result && !result.discovering && !result.discovery_error) {
        announce(result.devices.length ? `发现 ${result.devices.length} 台音响` : '暂未发现音响，请检查网络或手动添加');
      }
    } catch { /* Native discovery failure is also kept in the persistent error region. */ }
    finally { discoveryPending = false; renderControls(); await refresh(); }
  });
  function commitSearch() { query = $('search').value; page = 1; renderLibrary(); }
  $('search').value = query;
  $('search').addEventListener('compositionstart', () => { composing = true; });
  $('search').addEventListener('compositionend', () => { composing = false; commitSearch(); });
  $('search').addEventListener('input', (event) => { if (!composing && !event.isComposing) commitSearch(); });
  function clearSearch() { $('search').value = ''; query = ''; page = 1; renderLibrary(); $('search').focus(); }
  $('clear-search').addEventListener('click', clearSearch);
  $('page-previous').addEventListener('click', () => { page -= 1; renderLibrary(); });
  $('page-next').addEventListener('click', () => { page += 1; renderLibrary(); });

  $('open-device').addEventListener('click', () => {
    $('device-error').textContent = '';
    $('device-url').removeAttribute('aria-invalid');
    $('device-dialog').showModal();
    $('device-url').focus();
  });
  document.querySelectorAll('.dialog-close').forEach((button) => button.addEventListener('click', () => {
    if (!mutationBusy) button.closest('dialog').close();
  }));
  $('device-dialog').addEventListener('cancel', (event) => { if (mutationBusy) event.preventDefault(); });
  $('device-url').addEventListener('compositionstart', () => { deviceFormComposing = true; });
  $('device-url').addEventListener('compositionend', () => { deviceFormComposing = false; });
  function fieldError(message) {
    $('device-error').textContent = message;
    $('device-url').setAttribute('aria-invalid', 'true');
    $('device-url').focus();
  }
  $('device-form').addEventListener('submit', async (event) => {
    event.preventDefault();
    if (mutationBusy || deviceFormComposing) return;
    if (!connected || !config) { fieldError('客户端尚未就绪，请重新检查后再添加。'); return; }
    const url = $('device-url').value.trim();
    try { const parsed = new URL(url); if (!['http:', 'https:'].includes(parsed.protocol) || !parsed.hostname) throw new Error('invalid'); }
    catch { fieldError('请输入完整的 http 或 https 设备描述地址。'); return; }
    $('device-error').textContent = '';
    $('device-url').removeAttribute('aria-invalid');
    $('device-submit').setAttribute('aria-busy', 'true');
    $('device-dialog').querySelectorAll('button').forEach((button) => { button.disabled = true; });
    try {
      const result = await mutate('add_device', { url });
      if (result) { $('device-dialog').close(); announce('已添加音响，请在设备列表中选择'); await refresh(); }
    } catch (error) { fieldError(messageOf(error)); }
    finally {
      $('device-submit').setAttribute('aria-busy', 'false');
      $('device-dialog').querySelectorAll('button').forEach((button) => { button.disabled = false; });
    }
  });
  async function importMusic(folder) {
    if (!connected || !config || mutationBusy || importPending) return;
    importPending = true;
    $('import-status').hidden = false;
    $('import-status').classList.add('is-pending');
    $('import-status-title').textContent = folder ? '选择音乐文件夹' : '选择本地音乐';
    $('import-status-copy').textContent = '请在系统对话框中选择。导入会读取原文件，不会复制或上传音乐。';
    const button = $(folder ? 'open-folder' : 'add-music');
    button.setAttribute('aria-busy', 'true');
    renderControls();
    let nativeSlowTimer;
    // Native pickers are user-controlled and must not time out while choosing.
    try {
      generation += 1;
      actionError = '';
      renderErrors();
      nativeSlowTimer = setTimeout(() => {
        $('import-status-copy').textContent = '等待文件选择或导入完成。可在系统选择对话框中取消；大文件夹可能需要一些时间。';
      }, 15000);
      const result = await invoke(folder ? 'pick_music_folder' : 'pick_music_files');
      if (result === null) { $('import-status').hidden = true; return; }
      const added = Number(result.added) || 0;
      const skipped = Number(result.skipped) || 0;
      $('import-status-title').textContent = added ? `已加入 ${localeNumber.format(added)} 首音乐` : '未添加新的音乐';
      const warnings = Array.isArray(result.warnings) ? result.warnings.filter(Boolean) : [];
      $('import-status-copy').textContent = [`曲库共 ${localeNumber.format(result.total || 0)} 首`, skipped ? `跳过 ${localeNumber.format(skipped)} 个文件` : '', ...warnings].filter(Boolean).join('。');
      announce(added ? `已加入 ${localeNumber.format(added)} 首音乐` : '已检查所选音乐，没有新增文件');
    } catch (error) {
      actionError = messageOf(error);
      $('import-status-title').textContent = '导入未完成';
      $('import-status-copy').textContent = `${actionError} 可重新选择文件或文件夹再试。`;
      renderErrors();
    } finally {
      clearTimeout(nativeSlowTimer);
      importPending = false;
      button.setAttribute('aria-busy', 'false');
      $('import-status').classList.remove('is-pending');
      renderControls();
      await refresh();
    }
  }
  $('add-music').addEventListener('click', () => importMusic(false));
  $('open-folder').addEventListener('click', () => importMusic(true));
  $('empty-add').addEventListener('click', () => query ? clearSearch() : importMusic(false));
  async function connect() {
    if (connecting || !desktopAvailable) return;
    connecting = true;
    $('retry-connection').disabled = true;
    try { await loadConfig(); await refresh(); }
    catch (error) { setConnected(false, messageOf(error)); renderLibrary(); }
    finally { connecting = false; $('retry-connection').disabled = false; }
  }
  $('retry-connection').addEventListener('click', connect);
  // Shared service seam for extension dialogs; no second notification system.
  window.MusicUI = { invoke, mutate, announce, refresh, messageOf, formatSize, formatTime, element };
  // Controls only become actionable once their handlers have been attached.
  document.querySelectorAll('button').forEach((button) => { button.disabled = false; });
  renderLibrary();
  renderControls();
  if (!desktopAvailable) {
    setConnected(false, '请从已安装的桌面客户端打开。此界面需要客户端提供音乐文件访问和音响控制，无法单独在浏览器中运行。');
    return;
  }
  connect();
  setInterval(async () => {
    if (!config && !connecting) await connect();
    else await refresh();
  }, 2000);
})();
