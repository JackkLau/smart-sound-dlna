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
    if (!token) { preview = null; $('download-form').hidden = true; $('jobs-panel').hidden = true; }
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
      if (error.name === 'AbortError' || error instanceof TypeError) throw new Error('暂时无法连接电脑。请确认同一 Wi-Fi、电脑应用仍在运行，且“手机发送”已开启。连接恢复后可重试。');
      throw error;
    } finally { clearTimeout(timeout); }
  }
  function form(id, action) {
    let composing = false;
    $(id).addEventListener('compositionstart', () => { composing = true; });
    $(id).addEventListener('compositionend', () => { composing = false; });
    $(id).addEventListener('submit', async (event) => {
      event.preventDefault();
      if (busy || composing) return;
      busy = true;
      const error = $(`${id.replace('-form', '')}-error`);
      error.textContent = '';
      const controls = [...document.querySelectorAll('button,input,textarea')];
      controls.forEach((control) => { control.disabled = true; });
      const button = $(id).querySelector('button[type=submit]');
      const label = button.textContent;
      button.textContent = '正在处理…'; button.setAttribute('aria-busy', 'true');
      try { await action(); }
      catch (e) { error.textContent = e.message; }
      finally {
        busy = false;
        controls.forEach((control) => { control.disabled = false; });
        button.textContent = label; button.setAttribute('aria-busy', 'false');
      }
    });
  }
  async function restore() {
    if (restoring || busy) return;
    restoring = true;
    $('connection-retry').disabled = true;
    $('connection-retry').setAttribute('aria-busy', 'true');
    $('pair-status').textContent = token ? '正在恢复与电脑的连接…' : ticket ? '正在自动配对…' : '首次使用请扫描电脑“手机发送”中的二维码，无需输入连接码。';
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
    if (!ready || !token || polling || document.hidden) return;
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
    } catch (error) { $('connection-error').textContent = error.message; }
    finally { polling = false; }
  }
  restore(); setInterval(poll, 3000);
  document.addEventListener('visibilitychange', () => { if (!document.hidden) { if (ready) poll(); else restore(); } });
})();
