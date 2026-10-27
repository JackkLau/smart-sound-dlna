'use strict';
(() => {
  const $ = (id) => document.getElementById(id);
  const phases = { queued: '等待下载', resolving: '正在获取音轨', downloading: '正在下载', importing: '正在加入曲库', completed: '已加入电脑曲库', failed: '下载未完成', cancelling: '正在取消', cancelled: '已取消' };
  let token = '';
  let preview = null;
  let busy = false;
  let polling = false;
  let ready = false;
  let restoring = false;
  let libraryData = null;
  let player = null;
  let playbackFresh = false;
  let libraryVersion = 0;
  let query = '';
  let page = 1;
  let searchComposing = false;
  let searchTimer;
  let modeDirty = false;
  let listSignature = '';
  let ticket = new URLSearchParams(location.hash.slice(1)).get('pair') || '';
  // A one-use ticket is never retained in bookmarks or sent in an HTTP URL.
  if (location.hash) history.replaceState(null, '', location.pathname + location.search);
  try { token = localStorage.getItem('mobile-session-v1') || ''; } catch (_) { /* Memory remains usable. */ }
  function connected(value) {
    token = value;
    ready = Boolean(token);
    try { if (value) localStorage.setItem('mobile-session-v1', value); else localStorage.removeItem('mobile-session-v1'); }
    catch (_) { $('remember-note').textContent = '浏览器未允许保存配对。本次可以发送；下次需要在电脑忘记手机后重新扫码。请使用常用浏览器的普通模式。'; }
    $('pair-panel').hidden = Boolean(token);
    $('send-panel').hidden = !token;
    $('listen-panel').hidden = !token;
    if (!token) { libraryVersion++; libraryData = null; player = null; $('phone-tracks').replaceChildren(); listSignature = ''; playbackFresh = false; preview = null; $('download-form').hidden = true; $('jobs-panel').hidden = true; }
  }
  async function api(path, body) {
    const controller = new AbortController();
    const timeout = setTimeout(() => controller.abort(), 120000);
    try {
      const response = await fetch(`/api/${path}`, {
        method: body === undefined ? 'GET' : 'POST',
        headers: { 'Content-Type': 'application/json', ...(token ? { Authorization: `Bearer ${token}` } : {}) },
        ...(body === undefined ? {} : { body: JSON.stringify(body) }), signal: controller.signal, cache: 'no-store',
      });
      const data = await response.json().catch(() => ({}));
      if (!response.ok) {
        if (response.status === 401 && path !== 'pair') { connected(''); $('pair-status').textContent = '配对已失效，请扫描电脑的新二维码。'; }
        throw new Error(data.error || `电脑未能处理请求（${response.status}），请重试。`);
      }
      $('connection-error').textContent = '';
      return data;
    } catch (error) {
      if (error.name === 'AbortError' || error instanceof TypeError) throw new Error('暂时无法连接电脑。请确认同一 Wi-Fi、电脑应用仍在运行，且“手机点歌 / 导入”已开启。连接恢复后可重试。');
      throw error;
    } finally { clearTimeout(timeout); }
  }
  function renderPlayback() {
    const usable = ready && playbackFresh && Boolean(player) && !busy;
    const speaker = usable && player.connection === 'connected' && Boolean(player.device_id);
    $('phone-mode').disabled = !usable;
    $('mode-form').querySelector('button[type=submit]').disabled = !usable;
    if (player) {
      if (!modeDirty && document.activeElement !== $('phone-mode')) $('phone-mode').value = player.mode;
      const labels = { PLAYING: '正在播放', PAUSED_PLAYBACK: '已暂停', TRANSITIONING: '正在准备播放', STOPPED: '已停止', NO_MEDIA_PRESENT: '已停止' };
      $('speaker-status').textContent = player.connection === 'connected' ? `${player.device_name} · ${labels[player.transport] || '等待播放'}` : player.connection === 'disconnected' ? '音响已断开，电脑正在重连' : '请先在电脑选择并连接音响';
      $('playing-title').textContent = player.title || '还没有播放本地音乐';
      const mode = { sequential: '顺序播放', 'repeat-all': '列表循环', 'repeat-one': '单曲循环' }[player.mode];
      $('phone-queue-help').textContent = `${mode} · 按电脑完整曲库排列，搜索只用于找歌。${modeDirty ? '新模式尚未应用。' : ''}`;
    }
    for (const id of ['phone-previous', 'phone-next', 'phone-play-all']) $(id).disabled = !speaker || !libraryData?.library_total;
    $('phone-toggle').disabled = !speaker || !libraryData?.library_total || player.transport === 'TRANSITIONING';
    $('phone-toggle').textContent = player?.transport === 'PLAYING' && player?.current_track_id ? '暂停' : player?.transport === 'PAUSED_PLAYBACK' && player?.current_track_id ? '继续播放' : '播放';
    $('phone-stop').disabled = !speaker || !player?.current_track_id || ['STOPPED', 'NO_MEDIA_PRESENT'].includes(player?.transport);
    $('phone-tracks').querySelectorAll('button').forEach((button) => { button.disabled = !speaker; });
    $('phone-page-previous').disabled = !usable || !libraryData || page <= 1;
    $('phone-page-next').disabled = !usable || !libraryData || page >= libraryData.pages;
    $('library-retry').disabled = busy;
    $('phone-clear').disabled = busy;
    $('phone-clear').hidden = !$('phone-search').value;
  }
  async function action(button, error, callback) {
    if (busy || restoring) return;
    busy = true; libraryVersion++;
    error.textContent = '';
    const controls = [...document.querySelectorAll('button,input,textarea,select')];
    controls.forEach((control) => { control.disabled = true; });
    button.setAttribute('aria-busy', 'true');
    try { await callback(); }
    catch (e) { error.textContent = e.message; }
    finally {
      busy = false;
      controls.forEach((control) => { control.disabled = false; });
      button.setAttribute('aria-busy', 'false');
      renderPlayback();
      await poll();
    }
  }
  function form(id, callback) {
    let composing = false;
    $(id).addEventListener('compositionstart', () => { composing = true; });
    $(id).addEventListener('compositionend', () => { composing = false; });
    $(id).addEventListener('submit', (event) => {
      event.preventDefault();
      if (composing) return;
      return action($(id).querySelector('button[type=submit]'), $(`${id.replace('-form', '')}-error`), callback);
    });
  }
  async function loadLibrary() {
    if (!ready || busy || searchComposing) return;
    const version = ++libraryVersion;
    let data;
    try { data = await api(`library?${new URLSearchParams({ q: query, page: String(page) })}`); }
    catch (error) { if (version !== libraryVersion) return; throw error; }
    if (version !== libraryVersion || !ready || busy) return;
    libraryData = data; player = data.player; playbackFresh = true; page = data.page;
    const signature = JSON.stringify([data.tracks, player.current_track_id]);
    if (signature !== listSignature) {
      listSignature = signature;
      const focused = document.activeElement?.dataset?.trackId;
      $('phone-tracks').replaceChildren(...data.tracks.map((track) => {
        const item = document.createElement('li'); const button = document.createElement('button');
        button.type = 'button'; button.className = 'mobile-track'; button.dataset.trackId = track.id;
        button.setAttribute('aria-current', track.id === player.current_track_id ? 'true' : 'false');
        button.setAttribute('aria-label', `播放 ${track.title}`);
        const title = document.createElement('strong'); title.textContent = track.title;
        const detail = document.createElement('span'); detail.textContent = `${track.format.toUpperCase()} · ${track.id === player.current_track_id ? '当前歌曲' : '点击播放'}`;
        button.append(title, detail); item.append(button);
        button.addEventListener('click', () => sendPlayback(button, { action: 'play', track_id: track.id }));
        return item;
      }));
      if (focused) [...$('phone-tracks').querySelectorAll('button')].find((b) => b.dataset.trackId === focused)?.focus({ preventScroll: true });
    }
    $('library-status').textContent = data.total ? `共 ${data.total} 首 · 当前第 ${(page - 1) * 20 + 1}–${Math.min(page * 20, data.total)} 首` : query ? '没有找到歌曲，请换一个曲名或清空搜索。' : '曲库为空，请先在电脑添加音乐，或在下方导入视频音轨。';
    $('phone-page-info').textContent = `${page} / ${data.pages}`;
    renderPlayback();
  }
  function sendPlayback(button, payload) {
    return action(button, $('listen-error'), async () => {
      if (!player?.device_id) throw new Error('请先在电脑选择并连接音响。');
      player = await api('playback', { ...payload, device_id: player.device_id });
      renderPlayback();
    });
  }
  form('mode-form', async () => {
    player = await api('playback', { action: 'mode', mode: $('phone-mode').value });
    modeDirty = false;
  });
  $('phone-mode').addEventListener('change', () => { modeDirty = true; renderPlayback(); });
  for (const command of ['previous', 'next', 'stop']) $(`phone-${command}`).addEventListener('click', () => sendPlayback($(`phone-${command}`), { action: 'control', command }));
  $('phone-play-all').addEventListener('click', () => sendPlayback($('phone-play-all'), { action: 'play-all' }));
  $('phone-toggle').addEventListener('click', () => {
    const command = player?.transport === 'PLAYING' ? 'pause' : player?.transport === 'PAUSED_PLAYBACK' ? 'resume' : null;
    if (command && player?.current_track_id) sendPlayback($('phone-toggle'), { action: 'control', command });
    else if (player?.current_track_id) sendPlayback($('phone-toggle'), { action: 'play', track_id: player.current_track_id });
    else sendPlayback($('phone-toggle'), { action: 'play-all' });
  });
  function search() {
    clearTimeout(searchTimer); query = $('phone-search').value.trim(); page = 1; libraryVersion++;
    $('phone-clear').hidden = !$('phone-search').value;
    searchTimer = setTimeout(() => loadLibrary().catch((e) => { $('listen-error').textContent = e.message; }), 300);
  }
  $('phone-search').addEventListener('compositionstart', () => { searchComposing = true; libraryVersion++; clearTimeout(searchTimer); });
  $('phone-search').addEventListener('compositionend', () => { searchComposing = false; search(); });
  $('phone-search').addEventListener('input', () => { if (!searchComposing) search(); });
  $('phone-clear').addEventListener('click', () => { clearTimeout(searchTimer); $('phone-search').value = ''; query = ''; page = 1; loadLibrary().catch((e) => { $('listen-error').textContent = e.message; }); $('phone-search').focus(); });
  for (const [id, delta] of [['phone-page-previous', -1], ['phone-page-next', 1]]) $(id).addEventListener('click', () => { page += delta; loadLibrary().catch((e) => { $('listen-error').textContent = e.message; }); });
  $('library-retry').addEventListener('click', () => poll());
  async function restore() {
    if (restoring || busy) return;
    restoring = true;
    $('connection-retry').disabled = true;
    $('connection-retry').setAttribute('aria-busy', 'true');
    $('pair-status').textContent = token ? '正在恢复与电脑的连接…' : ticket ? '正在自动配对…' : '首次使用请扫描电脑“手机点歌 / 导入”中的二维码，无需输入连接码。';
    try {
      if (token) {
        try { await api('jobs'); connected(token); ticket = ''; return; }
        catch (error) { if (token || !ticket) throw error; }
      }
      if (ticket) {
        const result = await api('pair', { ticket });
        connected(result.token); ticket = '';
      }
    } catch (error) { $('connection-error').textContent = error.message; }
    finally {
      restoring = false;
      $('connection-retry').disabled = false;
      $('connection-retry').setAttribute('aria-busy', 'false');
      if (ready) poll();
    }
  }
  $('connection-retry').addEventListener('click', restore);
  form('parse-form', async () => {
    if (!$('share-text').value.trim()) throw new Error('请先粘贴 Bilibili 视频分享链接。');
    preview = null; $('download-form').hidden = true; $('download-summary').textContent = ''; $('download-error').textContent = '';
    preview = await api('parse', { text: $('share-text').value });
    $('video-title').textContent = preview.title;
    $('video-author').textContent = `${preview.author} · ${preview.parts.length} 个音轨`;
    $('parts').querySelectorAll('label').forEach((label) => label.remove());
    for (const part of preview.parts) {
      const label = document.createElement('label'); label.className = 'mobile-part';
      const input = document.createElement('input'); input.type = 'checkbox'; input.value = String(part.cid); input.checked = part.page === preview.selected_page;
      const text = document.createElement('span'); text.textContent = `P${part.page} · ${part.title}`;
      label.append(input, text); $('parts').append(label);
    }
    $('download-form').hidden = false; count();
    $('video-title').scrollIntoView({ block: 'nearest' });
  });
  const selected = () => [...$('parts').querySelectorAll('input:checked')];
  function count() { $('selected-count').textContent = `已选 ${selected().length} 首（最多 100 首）`; }
  $('parts').addEventListener('change', count);
  form('download-form', async () => {
    if (!preview) throw new Error('请先解析视频。');
    const cids = selected().map((input) => Number(input.value));
    if (!cids.length || cids.length > 100) throw new Error('请选择 1 至 100 首音轨。');
    const result = await api('download', { preview_id: preview.id, cids });
    const accepted = new Set(result.items.filter((item) => item.accepted).map((item) => item.cid));
    selected().forEach((input) => { if (accepted.has(Number(input.value))) input.checked = false; });
    count();
    $('download-summary').textContent = `已发送 ${accepted.size} 首，电脑将下载并加入曲库。`;
    $('download-error').textContent = result.items.filter((item) => item.error).map((item) => `${item.title}：${item.error}`).join('\n');
    await poll();
  });
  async function poll() {
    if (!ready || !token || busy || polling || document.hidden) return;
    polling = true;
    try {
      const data = await api('jobs');
      $('jobs-panel').hidden = !data.jobs.length;
      $('jobs').replaceChildren(...data.jobs.map((job) => {
        const li = document.createElement('li'); const title = document.createElement('strong'); title.textContent = job.title;
        const status = document.createElement('p'); status.textContent = `${phases[job.phase] || job.phase}${job.phase === 'downloading' && job.total ? ` · ${Math.floor(job.downloaded / job.total * 100)}%` : ''}`;
        li.append(title, status);
        if (job.error) { const error = document.createElement('p'); error.className = 'field-error'; error.textContent = job.error; li.append(error); }
        return li;
      }));
      await loadLibrary();
    } catch (error) { playbackFresh = false; $('connection-error').textContent = error.message; renderPlayback(); }
    finally { polling = false; }
  }
  restore(); setInterval(poll, 3000);
  document.addEventListener('visibilitychange', () => { if (!document.hidden) { if (ready) poll(); else restore(); } });
})();
