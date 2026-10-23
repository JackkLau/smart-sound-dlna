'use strict';

(() => {
  const $ = (id) => document.getElementById(id);
  const PAGE_SIZE = 20;
  const localeNumber = new Intl.NumberFormat('zh-CN');
  const params = new URLSearchParams(location.search);
  let query = params.get('q') || '';
  let page = Math.max(1, Number.parseInt(params.get('page') || '1', 10) || 1);
  let composing = false;
  let config = null;
  let state = { tracks: [], devices: [], selected_device_id: null, current_track_id: null, transport_state: 'STOPPED', position: 0, duration: 0, volume: null, queue_mode: 'sequential', warnings: [] };
  let loaded = false;
  let online = false;
  let mutationBusy = false;
  let pollController = null;
  let polling = false;
  let connecting = false;
  let generation = 0;
  let toastTimer = null;
  let actionError = '';
  let trackSignature = '';
  let deviceSignature = '';
  let uploadItems = [];
  let uploading = false;
  let uploadSerial = 0;
  let activeUpload = null;
  let dragDepth = 0;

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
  function announce(message) {
    clearTimeout(toastTimer);
    $('toast').textContent = message;
    $('toast').hidden = false;
    toastTimer = setTimeout(() => { $('toast').hidden = true; }, 5500);
  }
  $('toast').addEventListener('mouseenter', () => clearTimeout(toastTimer));
  $('toast').addEventListener('mouseleave', () => { toastTimer = setTimeout(() => { $('toast').hidden = true; }, 3500); });
  function formatSize(bytes) {
    if (bytes < 1048576) return `${(bytes / 1024).toFixed(0)} KB`;
    return `${(bytes / 1048576).toFixed(1)} MB`;
  }
  function formatTime(seconds) {
    const value = Math.max(0, Math.floor(Number(seconds) || 0));
    const hours = Math.floor(value / 3600);
    return hours ? `${hours}:${String(Math.floor(value / 60) % 60).padStart(2, '0')}:${String(value % 60).padStart(2, '0')}` : `${Math.floor(value / 60)}:${String(value % 60).padStart(2, '0')}`;
  }
  function rememberSearch() {
    const url = new URL(location.href);
    if (query) url.searchParams.set('q', query); else url.searchParams.delete('q');
    if (page > 1) url.searchParams.set('page', String(page)); else url.searchParams.delete('page');
    history.replaceState(null, '', url);
  }
  function setOnline(value, message = '') {
    online = value;
    $('connection-status').className = `connection ${value ? 'online' : 'offline'}`;
    $('connection-status').replaceChildren(element('span', 'status-dot'), document.createTextNode(value ? '本地服务已连接' : '本地服务未连接'));
    $('connection-error').hidden = value;
    $('connection-error-text').textContent = message || '无法连接后台程序。请确认程序仍在运行，再重新连接。';
    renderControls();
  }
  async function request(path, options = {}, timeout = 15000) {
    const controller = options.signal ? null : new AbortController();
    const timer = controller ? setTimeout(() => controller.abort(), timeout) : null;
    try {
      const response = await fetch(path, { cache: 'no-store', ...options, signal: options.signal || controller.signal });
      let data;
      try { data = await response.json(); } catch { throw new Error('后台返回了无法读取的数据，请重新连接。'); }
      if (!response.ok) {
        const error = new Error(data.error || `请求未完成（${response.status}），请重试。`);
        error.status = response.status;
        throw error;
      }
      return data;
    } catch (error) {
      if (error.name === 'AbortError' && controller) throw new Error('请求等待超时。操作可能已完成，请检查当前状态后再试。');
      if (error instanceof TypeError) throw new Error('连接中断。请确认后台程序和局域网连接，再重试。');
      throw error;
    } finally { if (timer) clearTimeout(timer); }
  }
  async function loadConfig() {
    config = await request('/api/config');
    const extensions = config.formats || [];
    $('file-input').accept = extensions.map((extension) => `.${extension}`).join(',');
    $('formats').textContent = `${extensions.map((extension) => extension.toUpperCase()).join(' · ')} · 每首最大 ${formatSize(config.max_upload_bytes)}`;
  }
  async function refresh() {
    if (polling || mutationBusy) return;
    polling = true;
    pollController = new AbortController();
    const ownController = pollController;
    const currentGeneration = generation;
    const timer = setTimeout(() => ownController.abort(), 12000);
    try {
      const data = await request('/api/state', { signal: ownController.signal });
      if (currentGeneration === generation) {
        acceptState(data);
        setOnline(true);
      }
    } catch (error) {
      if (currentGeneration === generation && (!ownController.signal.aborted || !mutationBusy)) {
        setOnline(false, error.name === 'AbortError' ? '后台响应超时。将自动尝试重新连接，也可点击重新连接。' : error.message);
        if (!loaded) renderLibrary();
      }
    } finally {
      clearTimeout(timer);
      if (pollController === ownController) pollController = null;
      polling = false;
    }
  }
  async function mutate(path, body, successMessage = '') {
    if (mutationBusy || !config || !online) return null;
    mutationBusy = true;
    generation += 1;
    pollController?.abort();
    actionError = '';
    renderControls();
    renderErrors();
    try {
      const data = await request(path, { method: 'POST', headers: { 'Content-Type': 'application/json', 'X-Control-Token': config.control_token }, body: JSON.stringify(body) }, path.includes('discover') || path.includes('scan') ? 60000 : 20000);
      if (Array.isArray(data.tracks)) acceptState(data);
      if (successMessage) announce(successMessage);
      return data;
    } catch (error) {
      if (error.status === 403) {
        // A restarted local server uses a fresh token; never replay the failed action.
        try { await loadConfig(); } catch { /* The existing action error remains actionable. */ }
      }
      actionError = error.message;
      renderErrors();
      throw error;
    } finally {
      mutationBusy = false;
      renderControls();
    }
  }
  function acceptState(data) {
    state = { ...state, ...data, tracks: data.tracks || [], devices: data.devices || [] };
    loaded = true;
    renderLibrary();
    renderDevices();
    renderControls();
    renderErrors();
  }
  function renderErrors() {
    const messages = [actionError, state.status_error, ...(Array.isArray(state.warnings) ? state.warnings : [])].filter(Boolean);
    $('playback-error').hidden = !messages.length;
    $('playback-error-text').textContent = [...new Set(messages)].join(' ');
  }
  function renderLibrary() {
    const normalizedQuery = query.toLocaleLowerCase('zh-CN').trim();
    const tracks = state.tracks.filter((track) => `${track.title} ${track.filename}`.toLocaleLowerCase('zh-CN').includes(normalizedQuery));
    const pages = Math.max(1, Math.ceil(tracks.length / PAGE_SIZE));
    page = Math.min(Math.max(1, page), pages);
    rememberSearch();
    $('track-count').textContent = localeNumber.format(state.tracks.length);
    $('clear-search').hidden = !query;
    $('page-info').textContent = `${page} / ${pages}`;
    $('page-previous').disabled = page <= 1;
    $('page-next').disabled = page >= pages;
    $('list-status').textContent = !loaded ? (online ? '正在读取音乐库…' : '音乐库尚未连接') : tracks.length ? `第 ${localeNumber.format((page - 1) * PAGE_SIZE + 1)}–${localeNumber.format(Math.min(page * PAGE_SIZE, tracks.length))} 首，共 ${localeNumber.format(tracks.length)} 首` : query ? '没有匹配的音乐' : '还没有添加音乐';
    $('empty-state').hidden = tracks.length > 0;
    if (!tracks.length) {
      $('empty-title').textContent = !loaded ? (online ? '正在读取音乐库' : '等待本地服务连接') : query ? '没有找到这首歌' : '这里，留给你的好音乐';
      $('empty-copy').textContent = !loaded ? '请保持后台程序运行，连接后将显示你的音乐。' : query ? `试试其他歌名或文件名，或清空「${query}」。` : '添加本地音乐，或导入整个文件夹。连接音响后，就能把音乐送到耳边。';
      $('empty-add').querySelector('span').textContent = query ? '清空搜索' : '添加第一首音乐';
      $('empty-add').disabled = !query && (!online || !config);
      $('formats').hidden = Boolean(query) || !loaded;
    }
    const slice = tracks.slice((page - 1) * PAGE_SIZE, page * PAGE_SIZE);
    const signature = JSON.stringify([slice, state.current_track_id, page]);
    if (signature !== trackSignature) {
      trackSignature = signature;
      const focusTrack = document.activeElement?.dataset?.trackId;
      const rows = slice.map((track, index) => {
        const row = element('li', `track-row${track.id === state.current_track_id ? ' current' : ''}`);
        const main = element('div', 'track-main');
        const number = element('span', 'track-number', track.id === state.current_track_id ? '♪' : String((page - 1) * PAGE_SIZE + index + 1).padStart(2, '0'));
        const text = element('div', 'track-text');
        text.append(element('strong', 'track-title', track.title || track.filename));
        text.append(element('span', 'track-filename', track.filename));
        main.append(number, text);
        const play = element('button', 'icon-button track-play');
        play.type = 'button';
        play.dataset.trackId = track.id;
        play.setAttribute('aria-label', `播放 ${track.title || track.filename}`);
        play.title = `播放 ${track.title || track.filename}`;
        play.append(icon('play'));
        play.addEventListener('click', () => playTrack(track.id));
        row.append(main, element('span', 'track-format', (track.format || '').toUpperCase()), element('span', 'track-size', formatSize(track.size || 0)), play);
        return row;
      });
      $('tracks').replaceChildren(...rows);
      if (focusTrack) [...$('tracks').querySelectorAll('button')].find((button) => button.dataset.trackId === focusTrack)?.focus({ preventScroll: true });
    }
    renderTrackControls();
  }
  function renderTrackControls() {
    const disabled = !online || !config || !state.selected_device_id || mutationBusy;
    document.querySelectorAll('.track-play').forEach((button) => { button.disabled = disabled; button.title = !state.selected_device_id ? '请先连接音响' : button.getAttribute('aria-label'); });
    $('library-caption').textContent = state.tracks.length && !state.selected_device_id ? '先连接音响，再播放音乐' : '本地音乐 · 直接推流';
  }
  function renderDevices() {
    const device = state.devices.find((item) => item.id === state.selected_device_id);
    $('device-title').textContent = device?.name || '连接你的音响';
    $('device-subtitle').textContent = device ? [device.model, device.address].filter(Boolean).join(' · ') || '已选择，随时可以播放' : '让喜欢的音乐，回到房间里。';
    document.querySelector('.speaker-art').classList.toggle('connected', Boolean(device));
    const signature = JSON.stringify([state.devices, state.selected_device_id]);
    if (signature === deviceSignature) return;
    deviceSignature = signature;
    const focusDevice = document.activeElement?.dataset?.deviceId;
    const buttons = state.devices.map((item) => {
      const button = element('button', 'device-option');
      button.type = 'button';
      button.dataset.deviceId = item.id;
      button.setAttribute('aria-pressed', String(item.id === state.selected_device_id));
      button.setAttribute('aria-label', `${item.id === state.selected_device_id ? '已选择' : '连接'} ${item.name}`);
      const label = element('span', 'device-label');
      label.append(element('strong', '', item.name), element('small', '', item.model || item.address || 'DLNA 音响'));
      button.append(icon('speaker'), label);
      if (item.id === state.selected_device_id) button.append(icon('check', 'small check'));
      button.addEventListener('click', async () => {
        if (item.id === state.selected_device_id) return;
        try { await mutate('/api/select', { device_id: item.id }, `已连接 ${item.name}`); } catch { /* Persistent error already shown. */ }
      });
      return button;
    });
    $('devices').replaceChildren(...buttons);
    if (focusDevice) [...$('devices').querySelectorAll('button')].find((button) => button.dataset.deviceId === focusDevice)?.focus({ preventScroll: true });
  }
  function renderControls() {
    const available = Boolean(online && config);
    const ready = available && Boolean(state.selected_device_id) && !mutationBusy;
    const current = state.tracks.find((track) => track.id === state.current_track_id);
    const device = state.devices.find((item) => item.id === state.selected_device_id);
    const playing = state.transport_state === 'PLAYING';
    const paused = state.transport_state === 'PAUSED_PLAYBACK';
    const transitioning = state.transport_state === 'TRANSITIONING';
    const transportLabels = { PLAYING: '正在播放', PAUSED_PLAYBACK: '已暂停', STOPPED: '已停止', TRANSITIONING: '正在连接音响…' };
    $('now-title').textContent = current ? current.title || current.filename : '还没有播放音乐';
    $('now-subtitle').textContent = current ? `${transportLabels[state.transport_state] || '准备播放'}${device ? ` · ${device.name}` : ''}` : device ? `播放到 ${device.name}` : '连接音响，选一首喜欢的歌';
    document.title = current ? `${playing ? '正在播放 · ' : ''}${current.title || current.filename} | 小米音响` : '小米音响 · 本地音乐';
    document.querySelector('.player').classList.toggle('is-playing', playing);
    $('play-icon').setAttribute('href', playing ? '#i-pause' : '#i-play');
    const playLabel = playing ? '暂停' : paused ? '继续播放' : '播放';
    $('play-pause').setAttribute('aria-label', playLabel);
    $('play-pause').title = !device ? '请先连接音响' : playLabel;
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
    $('mode').disabled = !available || mutationBusy;
    if (document.activeElement !== $('mode')) $('mode').value = state.queue_mode || 'sequential';
    ['discover', 'open-folder', 'open-device', 'add-music'].forEach((id) => { $(id).disabled = !available || mutationBusy; });
    document.querySelectorAll('.device-option').forEach((button) => { button.disabled = !available || mutationBusy; });
    $('empty-add').disabled = !query && (!available || mutationBusy);
    renderTrackControls();
  }
  async function playTrack(trackId) {
    try { await mutate('/api/play', { track_id: trackId }); } catch { /* Persistent error already shown. */ }
  }
  async function control(action, value) {
    try {
      await mutate('/api/control', { action, ...(value === undefined ? {} : { value }) });
    } catch { /* Persistent error already shown; restore server values on blur/poll. */ }
    await refresh();
  }
  $('play-pause').addEventListener('click', () => {
    if (state.transport_state === 'PLAYING') control('pause');
    else if (state.transport_state === 'PAUSED_PLAYBACK') control('resume');
    else if (state.tracks.length) playTrack(state.current_track_id || state.tracks[0].id);
  });
  ['previous', 'next', 'stop'].forEach((id) => $(id).addEventListener('click', () => control(id)));
  $('seek').addEventListener('input', () => { $('elapsed').textContent = formatTime($('seek').value); $('seek').setAttribute('aria-valuetext', `${formatTime($('seek').value)}，共 ${formatTime(state.duration)}`); });
  $('seek').addEventListener('change', () => control('seek', Number($('seek').value)));
  $('volume').addEventListener('input', () => { $('volume-label').textContent = $('volume').value; });
  $('volume').addEventListener('change', () => control('volume', Number($('volume').value)));
  $('mode').addEventListener('change', () => control('mode', $('mode').value));
  $('discover').addEventListener('click', async () => {
    $('discover').setAttribute('aria-busy', 'true');
    $('device-hint').textContent = '正在搜索，请稍等片刻…';
    try {
      const data = await mutate('/api/discover', {});
      if (data) {
        state.devices = data.devices || [];
        renderDevices();
        $('device-hint').textContent = data.devices.length ? `发现 ${data.devices.length} 台音响，点击即可连接` : '暂未发现音响，请检查网络或手动添加';
        announce(data.devices.length ? `发现 ${data.devices.length} 台音响` : '未发现音响，可通过设备地址添加');
      }
    } catch { $('device-hint').textContent = '搜索未完成，请重试或通过地址添加'; }
    finally { $('discover').setAttribute('aria-busy', 'false'); await refresh(); }
  });
  function commitSearch() {
    query = $('search').value;
    page = 1;
    renderLibrary();
  }
  $('search').value = query;
  $('search').addEventListener('compositionstart', () => { composing = true; });
  $('search').addEventListener('compositionend', () => { composing = false; commitSearch(); });
  $('search').addEventListener('input', (event) => { if (!composing && !event.isComposing) commitSearch(); });
  function clearSearch() { $('search').value = ''; query = ''; page = 1; renderLibrary(); $('search').focus(); }
  $('clear-search').addEventListener('click', clearSearch);
  $('page-previous').addEventListener('click', () => { page -= 1; renderLibrary(); });
  $('page-next').addEventListener('click', () => { page += 1; renderLibrary(); });

  function openDialog(id, fieldId, errorId) {
    $(errorId).textContent = '';
    $(fieldId).removeAttribute('aria-invalid');
    $(id).showModal();
    $(fieldId).focus();
  }
  $('open-folder').addEventListener('click', () => openDialog('folder-dialog', 'folder-path', 'folder-error'));
  $('open-device').addEventListener('click', () => openDialog('device-dialog', 'device-url', 'device-error'));
  document.querySelectorAll('.dialog-close').forEach((button) => button.addEventListener('click', () => { if (!mutationBusy) button.closest('dialog').close(); }));
  document.querySelectorAll('dialog').forEach((dialog) => dialog.addEventListener('cancel', (event) => { if (mutationBusy) event.preventDefault(); }));
  function fieldError(fieldId, errorId, message) {
    $(errorId).textContent = message;
    $(fieldId).setAttribute('aria-invalid', 'true');
    $(fieldId).focus();
  }
  async function submitDialog({ dialog, field, error, button, path, body, success }) {
    if (mutationBusy) return;
    if (!online || !config) { fieldError(field, error, '本地服务未连接。请先重新连接，再重试。'); return; }
    $(error).textContent = '';
    $(field).removeAttribute('aria-invalid');
    $(button).setAttribute('aria-busy', 'true');
    $(dialog).querySelectorAll('button').forEach((item) => { item.disabled = true; });
    try {
      const result = await mutate(path, body);
      if (result) { $(dialog).close(); announce(success(result)); await refresh(); }
    } catch (failure) { fieldError(field, error, failure.message); }
    finally { $(button).setAttribute('aria-busy', 'false'); $(dialog).querySelectorAll('button').forEach((item) => { item.disabled = false; }); }
  }
  $('folder-form').addEventListener('submit', (event) => {
    event.preventDefault();
    const value = $('folder-path').value.trim();
    if (!value) return fieldError('folder-path', 'folder-error', '请填写电脑上的文件夹完整路径。');
    submitDialog({ dialog: 'folder-dialog', field: 'folder-path', error: 'folder-error', button: 'folder-submit', path: '/api/library/scan', body: { path: value }, success: (result) => `已加入 ${result.added} 首音乐${result.skipped ? `，跳过 ${result.skipped} 个文件` : ''}` });
  });
  $('device-form').addEventListener('submit', (event) => {
    event.preventDefault();
    const value = $('device-url').value.trim();
    try { const url = new URL(value); if (!['http:', 'https:'].includes(url.protocol) || !url.hostname) throw new Error('invalid'); }
    catch { return fieldError('device-url', 'device-error', '请输入完整的 http 或 https 设备描述地址。'); }
    submitDialog({ dialog: 'device-dialog', field: 'device-url', error: 'device-error', button: 'device-submit', path: '/api/devices', body: { url: value }, success: () => '已添加音响，请在设备列表中选择' });
  });

  function chooseFiles() { if (config && online && !mutationBusy) $('file-input').click(); }
  $('add-music').addEventListener('click', chooseFiles);
  $('empty-add').addEventListener('click', () => query ? clearSearch() : chooseFiles());
  $('file-input').addEventListener('change', () => { addFiles($('file-input').files); $('file-input').value = ''; });
  const dropZone = $('drop-zone');
  dropZone.addEventListener('dragenter', (event) => { if (!event.dataTransfer.types.includes('Files')) return; event.preventDefault(); dragDepth += 1; dropZone.classList.add('dragging'); });
  dropZone.addEventListener('dragover', (event) => { if (event.dataTransfer.types.includes('Files')) event.preventDefault(); });
  dropZone.addEventListener('dragleave', () => { dragDepth = Math.max(0, dragDepth - 1); if (!dragDepth) dropZone.classList.remove('dragging'); });
  dropZone.addEventListener('drop', (event) => { event.preventDefault(); dragDepth = 0; dropZone.classList.remove('dragging'); addFiles(event.dataTransfer.files); });
  function addFiles(files) {
    if (!config || !online) { announce('请先连接本地服务，再添加音乐'); return; }
    const incoming = Array.from(files);
    if (!incoming.length) return;
    if (incoming.length + uploadItems.filter((item) => ['waiting', 'uploading'].includes(item.status)).length > 100) { announce('一次最多添加 100 个文件，也可以使用文件夹导入'); return; }
    if (!uploading) uploadItems = uploadItems.filter((item) => ['error', 'cancelled'].includes(item.status));
    for (const file of incoming) {
      const extension = file.name.split('.').pop().toLowerCase();
      let error = '';
      if (!config.formats.includes(extension)) error = '不支持此格式，请选择支持的音乐文件';
      else if (!file.size) error = '文件为空，请选择有效的音乐文件';
      else if (file.size > config.max_upload_bytes) error = `超过 ${formatSize(config.max_upload_bytes)}，可改用文件夹导入`;
      uploadItems.push({ id: ++uploadSerial, file, status: error ? 'invalid' : 'waiting', progress: 0, error });
    }
    $('upload-panel').hidden = false;
    renderUploads();
    runUploads();
  }
  function renderUploads() {
    const focusedUpload = document.activeElement?.dataset?.uploadId;
    $('upload-list').replaceChildren(...uploadItems.map((item) => {
      const row = element('li', 'upload-item');
      const labels = { waiting: '等待上传', uploading: `${item.progress}% · 正在上传`, success: '已加入音乐库', cancelled: '已取消', invalid: item.error, error: item.error };
      row.append(element('strong', '', item.file.name), element('span', ['error', 'invalid'].includes(item.status) ? 'upload-failure' : '', `${formatSize(item.file.size)} · ${labels[item.status]}`));
      if (['waiting', 'uploading', 'error', 'cancelled', 'invalid'].includes(item.status)) {
        const button = element('button', 'text-button', ['error', 'cancelled'].includes(item.status) ? '重试' : item.status === 'invalid' ? '移除' : '取消');
        button.type = 'button';
        button.dataset.uploadId = String(item.id);
        button.setAttribute('aria-label', `${button.textContent} ${item.file.name}`);
        button.addEventListener('click', () => {
          if (['error', 'cancelled'].includes(item.status)) { item.status = 'waiting'; item.error = ''; renderUploads(); runUploads(); }
          else if (item.status === 'invalid') { uploadItems = uploadItems.filter((entry) => entry.id !== item.id); renderUploads(); }
          else { item.status = 'cancelled'; if (activeUpload?.id === item.id) activeUpload.xhr.abort(); renderUploads(); }
        });
        row.append(button);
      }
      if (item.status === 'uploading') {
        const progress = element('progress');
        progress.max = 100;
        progress.value = item.progress;
        progress.setAttribute('aria-label', `${item.file.name} 上传进度`);
        row.append(progress);
      }
      return row;
    }));
    if (focusedUpload) [...$('upload-list').querySelectorAll('button')].find((button) => button.dataset.uploadId === focusedUpload)?.focus({ preventScroll: true });
    const pending = uploadItems.filter((item) => ['waiting', 'uploading'].includes(item.status)).length;
    const success = uploadItems.filter((item) => item.status === 'success').length;
    const errors = uploadItems.filter((item) => ['error', 'invalid'].includes(item.status)).length;
    $('upload-summary').textContent = `已加入 ${success} 首${pending ? `，还有 ${pending} 首` : ''}${errors ? `，${errors} 个文件需要处理` : ''}`;
    $('cancel-upload').textContent = pending ? '取消剩余上传' : '收起上传记录';
  }
  function uploadFile(item) {
    return new Promise((resolve, reject) => {
      const xhr = new XMLHttpRequest();
      activeUpload = { xhr, id: item.id };
      xhr.open('POST', '/api/library/upload');
      xhr.timeout = 600000;
      xhr.setRequestHeader('Content-Type', 'application/octet-stream');
      xhr.setRequestHeader('X-Control-Token', config.control_token);
      xhr.setRequestHeader('X-Filename', encodeURIComponent(item.file.name));
      xhr.upload.addEventListener('progress', (event) => {
        if (event.lengthComputable) {
          const progress = Math.round(event.loaded / event.total * 100);
          if (progress !== item.progress) { item.progress = progress; renderUploads(); }
        }
      });
      xhr.onload = () => {
        try {
          const result = JSON.parse(xhr.responseText);
          if (xhr.status >= 200 && xhr.status < 300) resolve(result);
          else { const error = new Error(result.error || '上传未完成，请重试'); error.status = xhr.status; reject(error); }
        } catch { reject(new Error('上传结果无法读取，请检查音乐库后重试')); }
      };
      xhr.onerror = () => reject(new Error('连接中断，请检查后台程序后重试'));
      xhr.ontimeout = () => reject(new Error('上传超时，请检查音乐库后重试'));
      xhr.onabort = () => reject(new DOMException('已取消', 'AbortError'));
      xhr.send(item.file);
    });
  }
  async function runUploads() {
    if (uploading) return;
    uploading = true;
    let item;
    while ((item = uploadItems.find((entry) => entry.status === 'waiting'))) {
      item.status = 'uploading';
      item.progress = 0;
      renderUploads();
      try { await uploadFile(item); item.status = 'success'; }
      catch (error) {
        item.status = error.name === 'AbortError' ? 'cancelled' : 'error';
        item.error = error.message;
        if (error.status === 403) { try { await loadConfig(); } catch { /* Keep the explicit retry path. */ } }
      }
      finally { activeUpload = null; renderUploads(); }
      await refresh();
    }
    uploading = false;
    renderUploads();
    await refresh();
  }
  $('cancel-upload').addEventListener('click', () => {
    const pending = uploadItems.some((item) => ['waiting', 'uploading'].includes(item.status));
    if (!pending) { $('upload-panel').hidden = true; return; }
    uploadItems.forEach((item) => { if (['waiting', 'uploading'].includes(item.status)) item.status = 'cancelled'; });
    activeUpload?.xhr.abort();
    renderUploads();
    announce('已取消剩余上传，已经加入的音乐会保留');
  });
  window.addEventListener('beforeunload', (event) => {
    if (uploading) { event.preventDefault(); event.returnValue = ''; }
  });
  async function connect() {
    if (connecting) return;
    connecting = true;
    $('retry-connection').disabled = true;
    try { await loadConfig(); await refresh(); }
    catch (error) { setOnline(false, error.message); renderLibrary(); }
    finally { connecting = false; $('retry-connection').disabled = false; }
  }
  $('retry-connection').addEventListener('click', connect);
  // Controls only become actionable after every event handler has been bound.
  document.querySelectorAll('button').forEach((button) => { button.disabled = false; });
  renderLibrary();
  renderControls();
  connect();
  setInterval(async () => {
    if (!config && !polling && !mutationBusy) { await connect(); }
    else await refresh();
  }, 2000);
})();
