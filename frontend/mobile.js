'use strict';
(() => {
  if (!window.MusicUI || !window.MusicFeatures) return;
  const { invoke, element } = window.MusicUI;
  const { featureAction, onSubmit, errorAt } = window.MusicFeatures;
  const $ = (id) => document.getElementById(id);
  let state = { enabled: false, entries: [] };
  let polling = false;
  let revision = 0;
  let signature = '';
  let qrUrl = '';
  function renderQr() {
    const entry = state.entries.find((entry) => entry.address === $('mobile-network').value) || state.entries[0];
    const url = state.enabled && (state.paired || state.pairing_available) ? entry?.url : '';
    $('mobile-qr-frame').hidden = !url;
    if (url === qrUrl) return;
    if (url) {
      const qr = window.qrcode(0, 'M');
      qr.addData(url); qr.make();
      $('mobile-qr').src = qr.createDataURL(6, 24);
    } else { $('mobile-qr').removeAttribute('src'); }
    qrUrl = url;
  }
  function render(next) {
    state = next;
    $('mobile-toggle').textContent = state.enabled ? '暂停手机接收' : '开启手机接收';
    $('mobile-details').hidden = !state.enabled;
    const nextSignature = JSON.stringify(state.entries.map((entry) => entry.address));
    if (signature !== nextSignature) {
      const selected = $('mobile-network').value;
      $('mobile-network').replaceChildren(...state.entries.map((entry) => {
        const option = element('option', '', entry.address); option.value = entry.address; option.selected = entry.address === selected; return option;
      }));
      signature = nextSignature;
    }
    $('mobile-network-field').hidden = state.entries.length < 2;
    $('mobile-status').textContent = state.paired
      ? '已记住手机。用配对时的浏览器打开收藏或桌面快捷方式，也可扫描下方二维码。换手机或浏览器时，请先忘记手机并重新配对。'
      : state.pairing_available ? '用手机相机扫码，在常用浏览器打开，自动完成配对，无需输入连接码。二维码 10 分钟内有效。'
        : '配对二维码已失效，请点击“忘记手机并重新配对”生成新的二维码。';
    renderQr();
  }
  async function poll() {
    if (polling || !$('mobile-dialog').open || $('mobile-dialog').dataset.busy === 'true') return;
    polling = true; const current = revision;
    try { const next = await invoke('get_mobile_receiver'); if (current === revision) render(next); }
    catch (error) { errorAt('mobile-form-error', error); }
    finally { polling = false; }
  }
  async function change(form, command, args, button) {
    revision += 1;
    await featureAction(form, '正在更新…', async () => { render(await invoke(command, args)); }, undefined, button);
    await poll();
  }
  $('open-mobile').addEventListener('click', () => { $('mobile-dialog').showModal(); poll(); });
  $('mobile-network').addEventListener('change', renderQr);
  onSubmit('mobile-form', (form) => change(form, 'set_mobile_receiver', { enabled: !state.enabled }));
  $('mobile-forget').addEventListener('click', () => change($('mobile-form'), 'forget_mobile_pairing', {}, $('mobile-forget')));
  setInterval(poll, 2000);
})();
