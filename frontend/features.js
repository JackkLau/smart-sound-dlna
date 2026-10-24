'use strict';

(() => {
  const ui = window.MusicUI;
  if (!ui || typeof ui.invoke !== 'function') return;
  const { invoke, announce, refresh, messageOf, formatSize, formatTime, element } = ui;
  const $ = (id) => document.getElementById(id);
  const phases = { queued: '等待下载', resolving: '正在获取音轨', downloading: '正在下载', cancelling: '正在取消', importing: '正在加入曲库', completed: '已加入曲库', failed: '下载未完成', cancelled: '已取消' };
  let features = null;
  let polling = false;
  let revision = 0;
  let deviceSignature = '';
  const jobs = new Map();
  const completed = new Set();
  const providerBusy = new Set();
  const loginErrors = new Map();
  const loginPhases = new Map();
  const busyForms = new Set();

  function errorAt(id, error, field) {
    $(id).textContent = error ? messageOf(error) : '';
    if (field) {
      $(field).setAttribute('aria-invalid', String(Boolean(error)));
      if (error) $(field).focus();
    }
  }
  async function pollFeatures() {
    if (polling) return;
    polling = true;
    const start = revision;
    try {
      const data = await invoke('get_features');
      if (start === revision) { features = data; render(); }
    } catch (error) {
      if ($('voice-dialog').open) errorAt('voice-runtime-error', error);
      if ($('download-panel').hidden === false) errorAt('download-error', error);
    } finally { polling = false; }
  }
  // Forms share pending behavior and preserve entered values on failure. The
  // native dialog can close whenever no foreground request is in progress.
  async function featureAction(form, pendingLabel, operation, field, pendingButton) {
    if (busyForms.has(form.id) || form.closest('dialog').dataset.busy === 'true') return;
    busyForms.add(form.id);
    revision += 1;
    errorAt(`${form.id}-error`, null, field);
    const dialog = form.closest('dialog');
    dialog.dataset.busy = 'true';
    const controls = [...form.querySelectorAll('button,input,select')];
    const previous = controls.map((control) => control.disabled);
    controls.forEach((control) => { control.disabled = true; });
    const submit = pendingButton || form.querySelector('[type="submit"]');
    const label = submit?.textContent;
    if (submit) { submit.textContent = pendingLabel; submit.setAttribute('aria-busy', 'true'); }
    try { return await operation(); }
    catch (error) { errorAt(`${form.id}-error`, error, field); return undefined; }
    finally {
      controls.forEach((control, index) => { control.disabled = previous[index]; });
      if (submit) { submit.textContent = label; submit.setAttribute('aria-busy', 'false'); }
      busyForms.delete(form.id);
      if (field && $(`${form.id}-error`).textContent) $(field).focus();
      dialog.dataset.busy = String([...busyForms].some((id) => $(id).closest('dialog') === dialog) || providerBusy.size > 0);
      revision += 1;
      await pollFeatures();
    }
  }
  function onSubmit(id, handler) {
    const form = $(id);
    let composing = false;
    form.addEventListener('compositionstart', () => { composing = true; });
    form.addEventListener('compositionend', () => { composing = false; });
    form.addEventListener('submit', (event) => { event.preventDefault(); if (!composing && !busyForms.has(id)) handler(form); });
  }
  document.querySelectorAll('.feature-dialog').forEach((dialog) => {
    dialog.addEventListener('cancel', (event) => { if (dialog.dataset.busy === 'true') event.preventDefault(); });
    dialog.querySelector('[data-feature-close]').addEventListener('click', () => { if (dialog.dataset.busy !== 'true') dialog.close(); });
  });
  $('open-bilibili').addEventListener('click', () => { $('bilibili-dialog').showModal(); document.querySelector('input[name=import-source]:checked')?.focus(); pollFeatures(); });
  $('open-voice').addEventListener('click', () => { $('voice-dialog').showModal(); pollFeatures(); });

  onSubmit('voice-bind-form', (form) => featureAction(form, '正在连接…', async () => {
    if (!features?.selected_device_id) throw new Error('请先在主界面选择播放音响。');
    if (!$('voice-source').value) throw new Error('请选择接收语音的小爱音响。');
    await invoke('enable_voice', { sourceId: $('voice-source').value, targetId: features.selected_device_id });
    announce('已开始读取新的本地点播指令');
  }, 'voice-source'));
  $('voice-stop').addEventListener('click', () => featureAction($('voice-bind-form'), '正在停止…', () => invoke('disable_voice')));
  onSubmit('voice-test-form', (form) => featureAction(form, '正在执行…', async () => {
    $('voice-test-result').textContent = '';
    const result = await invoke('voice_command', { text: $('voice-text').value });
    $('voice-test-result').textContent = result;
    await refresh();
  }, 'voice-text'));
  onSubmit('voice-session-form', (form) => featureAction(form, '正在连接…', async () => {
    const cookie = $('voice-cookie').value;
    // Do not leave secrets in the DOM after submitting them to the native layer.
    $('voice-cookie').value = '';
    await invoke('connect_voice_session', { cookie });
    announce('已连接小米账号，请选择接收语音的音响');
  }, 'voice-cookie'));

  async function accountAction(provider, action, button) {
    if (providerBusy.has(provider) || button.closest('dialog').dataset.busy === 'true') return;
    providerBusy.add(provider);
    revision += 1;
    const dialog = button.closest('dialog');
    dialog.dataset.busy = 'true';
    errorAt(`${provider}-login-error`, null);
    loginErrors.delete(provider);
    button.setAttribute('aria-busy', 'true');
    renderAccounts();
    try {
      await invoke(action, { provider });
    } catch (error) { loginErrors.set(provider, messageOf(error)); }
    finally {
      providerBusy.delete(provider);
      button.setAttribute('aria-busy', 'false');
      dialog.dataset.busy = String([...busyForms].some((id) => $(id).closest('dialog') === dialog));
      revision += 1; renderAccounts(); await pollFeatures();
    }
  }
  document.querySelectorAll('[data-login]').forEach((button) => button.addEventListener('click', () => accountAction(button.dataset.login, 'open_provider_login', button)));
  document.querySelectorAll('[data-finish-login]').forEach((button) => button.addEventListener('click', () => accountAction(button.dataset.finishLogin, 'finish_provider_login', button)));
  document.querySelectorAll('[data-disconnect]').forEach((button) => button.addEventListener('click', () => accountAction(button.dataset.disconnect, 'disconnect_provider', button)));
  function renderAccounts() {
    for (const provider of ['xiaomi', 'bilibili']) {
      const connected = provider === 'xiaomi' ? features?.voice.connected : features?.downloads.authenticated;
      const login = features?.[`${provider}_login`] || { phase: 'idle' };
      const pending = ['waiting', 'checking', 'failed'].includes(login.phase);
      const checking = login.phase === 'checking';
      const previousPhase = loginPhases.get(provider);
      if (previousPhase !== login.phase) {
        loginErrors.delete(provider);
        loginPhases.set(provider, login.phase);
        if (login.phase === 'connected') announce(provider === 'xiaomi' ? '已连接小米账号，请选择接收语音的音响' : 'Bilibili 登录成功');
      }
      const container = document.querySelector(`[data-provider="${provider}"]`);
      container.querySelectorAll('button').forEach((button) => { button.disabled = providerBusy.has(provider) || checking; });
      container.querySelector('[data-finish-login]').hidden = !pending && !(login.cached && !connected);
      container.querySelector('[data-disconnect]').hidden = !connected && !pending && !login.cached;
      container.querySelector('[data-finish-login]').textContent = checking ? '正在确认…' : '重试连接';
      container.querySelector('[data-login]').textContent = connected || login.cached ? '重新登录' : provider === 'xiaomi' ? '登录小米账号' : '登录 Bilibili';
      $(`${provider}-account-status`).textContent = checking ? login.cached ? '正在恢复已保存的登录，请稍候…' : '正在确认登录状态，请稍候…' : login.phase === 'waiting' ? '请在官方窗口完成登录，成功后窗口会自动关闭。' : connected ? login.cached ? '已连接 · 重启后自动恢复登录' : '已连接 · 登录状态尚未保存' : login.cached ? '登录状态已保存 · 暂未连接，可重试或重新登录' : login.phase === 'cancelled' ? '登录已取消，可重新打开登录窗口。' : provider === 'xiaomi' ? '未连接 · 使用绑定音响的小米账号' : '未登录 · 公开视频可直接解析';
      errorAt(`${provider}-login-error`, loginErrors.get(provider) || login.error);
    }
  }
  function render() {
    if (!features) return;
    renderAccounts();
    const voice = features.voice;
    $('voice-badge').textContent = voice.listening ? '接收中' : voice.connected ? '已连接' : '未连接';
    const signature = JSON.stringify(voice.devices);
    if (signature !== deviceSignature) {
      deviceSignature = signature;
      const selected = $('voice-source').value || voice.source_id;
      const options = voice.devices.map((device) => {
        const option = element('option', '', `${device.name} · ${device.hardware}`);
        option.value = device.id; option.selected = selected === device.id;
        return option;
      });
      if (!options.length) { const option = element('option', '', '登录后选择音响'); option.value = ''; options.push(option); }
      $('voice-source').replaceChildren(...options);
    }
    if (!busyForms.has('voice-bind-form')) {
      $('voice-source').disabled = !voice.connected;
      $('voice-start').disabled = !voice.connected || !features.selected_device_id;
      $('voice-start').textContent = voice.listening ? '更新绑定' : '开始接收指令';
      $('voice-stop').disabled = !voice.listening;
    }
    $('voice-target').textContent = `播放到：${features.selected_device_name || '请先在主界面连接音响'}${voice.listening && voice.target_id !== features.selected_device_id ? '（音响已切换，需更新绑定）' : ''}`;
    $('voice-status-title').textContent = voice.listening ? voice.last_poll_ms ? '正在接收本地点播指令' : '正在首次读取语音记录…' : '尚未开始接收';
    $('voice-status-copy').textContent = voice.last_command ? `「${voice.last_command}」：${voice.last_result}` : voice.listening ? '只处理启用后出现的新指令，历史对话不会执行。' : '使用下方测试入口，可先检查点歌指令。';
    $('voice-runtime-error').textContent = voice.error || '';
    $('download-directory').textContent = features.downloads.directory;
    renderJobs(features.downloads.jobs);
  }
  function renderJobs(list) {
    $('download-panel').hidden = !list.length;
    const live = new Set(list.map((job) => job.id));
    for (const [id, record] of jobs) { if (!live.has(id)) { record.row.remove(); jobs.delete(id); } }
    for (const job of list) {
      let record = jobs.get(job.id);
      if (!record) {
        const row = element('li', 'download-job');
        const content = element('div');
        const title = element('strong');
        const status = element('p');
        const progress = element('progress');
        const error = element('p', 'job-error'); error.setAttribute('role', 'status');
        const button = element('button', 'text-button'); button.type = 'button';
        content.append(title, status, progress, error); row.append(content, button);
        record = { row, title, status, progress, error, button, pending: false, job };
        jobs.set(job.id, record); $('download-jobs').prepend(row);
        button.addEventListener('click', async () => {
          if (record.pending) return;
          record.pending = true; button.disabled = true; errorAt('download-error', null);
          try { await invoke(['failed', 'cancelled'].includes(record.job.phase) ? 'retry_download' : 'cancel_download', { id: job.id }); }
          catch (error) { errorAt('download-error', error); }
          finally { record.pending = false; await pollFeatures(); }
        });
      }
      record.job = job;
      record.title.textContent = job.title;
      record.status.textContent = `${phases[job.phase] || job.phase}${job.downloaded ? ` · ${formatSize(job.downloaded)}${job.total ? ` / ${formatSize(job.total)}` : ''}` : ''}`;
      record.error.textContent = job.error || '';
      record.progress.hidden = !['downloading', 'resolving', 'importing'].includes(job.phase);
      record.progress.setAttribute('aria-label', `${job.title}：${phases[job.phase]}`);
      if (job.total && job.phase === 'downloading') { record.progress.max = job.total; record.progress.value = job.downloaded; }
      else record.progress.removeAttribute('value');
      record.button.hidden = ['completed', 'importing'].includes(job.phase);
      record.button.disabled = record.pending || job.phase === 'cancelling';
      record.button.textContent = ['failed', 'cancelled'].includes(job.phase) ? '重试' : '取消';
      record.button.setAttribute('aria-label', `${record.button.textContent} ${job.title}`);
      if (job.phase === 'completed' && !completed.has(job.id)) {
        completed.add(job.id); announce(`已加入曲库：${job.title}`); refresh();
      }
    }
  }
  window.MusicFeatures = { featureAction, onSubmit, errorAt, pollFeatures };
  pollFeatures();
  setInterval(pollFeatures, 2000);
})();
