const { test } = require('node:test');
const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const vm = require('node:vm');

function harness() {
  const nodes = new Map();
  const calls = [], announcements = [];
  const document = {
    getElementById: (id) => node(id),
    querySelectorAll: (selector) => selector === '.feature-dialog' ? [dialog] : selector === '.track-edit' ? [trigger] : [],
    querySelector: () => ({ querySelectorAll: () => [], querySelector: () => node('account-button') }),
  };
  function node(id) {
    if (!nodes.has(id)) nodes.set(id, {
      id, value: '', textContent: '', dataset: {}, handlers: {}, disabled: false, open: false, attrs: {},
      addEventListener(event, handler) { this.handlers[event] = handler; },
      setAttribute(name, value) { this.attrs[name] = value; },
      removeAttribute(name) { delete this.attrs[name]; },
      focus() { document.activeElement = this; }, select() {},
      replaceChildren() {}, closest() { return dialog; },
      querySelector(selector) { return selector === '[type="submit"]' ? submit : close; },
      querySelectorAll() { return controls; },
      showModal() { this.open = true; },
      close() { this.open = false; this.handlers.close?.(); },
    });
    return nodes.get(id);
  }
  const dialog = node('track-name-dialog');
  const form = node('track-name-form');
  const field = node('track-name');
  const submit = node('save'); submit.textContent = '保存曲名';
  const close = node('close');
  const trigger = node('edit'); trigger.dataset.trackId = 'one';
  const controls = [field, submit, node('recognize-track-name'), node('track-name-cancel')];
  let suggestion = { title: '晴天', source: 'audio_tag' };
  let recognition = null, renameError = null, renameWait = null;
  const track = { id: 'one', title: 'video title', filename: 'video.m4a' };
  const featureData = {
    voice: { connected: false, devices: [], listening: false },
    downloads: { authenticated: false, jobs: [] },
    xiaomi_login: { phase: 'idle' }, bilibili_login: { phase: 'idle' },
  };
  const window = { MusicUI: {
    invoke: async (command, args) => {
      calls.push({ command, args });
      if (command === 'get_features') return featureData;
      if (command === 'recognize_track_name') return recognition ? await recognition : suggestion;
      throw new Error(command);
    },
    mutate: async (command, args) => {
      calls.push({ command, args });
      if (renameWait) await renameWait;
      if (renameError) throw new Error(renameError);
      track.title = args.title;
      return { tracks: [track] };
    },
    announce: (message) => announcements.push(message), refresh() {},
    messageOf: (error) => error.message || String(error), formatSize: String, formatTime: String,
    element: () => ({ textContent: '' }),
  } };
  const context = vm.createContext({ window, document, setInterval() {} });
  for (const file of ['frontend/features.js', 'frontend/track-editor.js']) vm.runInContext(readFileSync(file, 'utf8'), context);
  const settle = () => new Promise(setImmediate);
  return {
    node, dialog, form, field, close, track, calls, announcements, document, settle,
    open() { window.MusicTrackEditor.open({ ...track }); },
    input(text) { field.value = text; field.handlers.input(); },
    submit() { form.handlers.submit({ preventDefault() {} }); },
    setSuggestion(value) { suggestion = value; },
    setRecognition(value) { recognition = value; },
    setRenameError(value) { renameError = value; },
    setRenameWait(value) { renameWait = value; },
  };
}

test('recognition edits a draft; save updates the spoken name and restores row focus', async () => {
  const ui = harness(); await ui.settle(); ui.open();
  assert.equal(ui.field.value, 'video title');
  ui.node('recognize-track-name').handlers.click(); await ui.settle();
  assert.equal(ui.field.value, '晴天');
  assert.equal(ui.track.title, 'video title');
  assert.match(ui.node('track-name-recognition').textContent, /音频标签/);
  assert.match(ui.node('track-name-voice').textContent, /播放本地 晴天/);
  ui.submit(); await ui.settle();
  const call = ui.calls.find((c) => c.command === 'rename_track');
  assert.deepEqual({ ...call.args }, { trackId: 'one', title: '晴天', expectedTitle: 'video title' });
  assert.equal(ui.dialog.open, false);
  assert.equal(ui.document.activeElement.id, 'edit');
  assert.equal(ui.track.title, '晴天');
  assert.match(ui.announcements[0], /播放本地 晴天/);
});

test('unknown recognition, cancel and validation keep user input without false success', async () => {
  const ui = harness(); await ui.settle(); ui.open();
  ui.input('自定曲名'); ui.setSuggestion({ title: null, source: 'filename' });
  ui.node('recognize-track-name').handlers.click(); await ui.settle();
  assert.equal(ui.field.value, '自定曲名');
  assert.match(ui.node('track-name-recognition').textContent, /未找到可靠曲名/);
  ui.node('track-name-cancel').handlers.click(); ui.open();
  assert.equal(ui.field.value, '自定曲名');
  ui.input('  '); ui.submit(); await ui.settle();
  assert.match(ui.node('track-name-form-error').textContent, /请输入/);
  assert.equal(ui.field.attrs['aria-invalid'], 'true');
  assert.equal(ui.document.activeElement, ui.field);
  assert.equal(ui.calls.filter((c) => c.command === 'rename_track').length, 0);
  assert.equal(ui.announcements.length, 0);
});

test('IME, duplicate submits, pending dismissal and failed save are handled by shared forms', async () => {
  const ui = harness(); await ui.settle(); ui.open(); ui.input('新曲名');
  ui.form.handlers.compositionstart(); ui.submit(); await ui.settle();
  assert.equal(ui.calls.filter((c) => c.command === 'rename_track').length, 0);
  ui.form.handlers.compositionend();
  let release; ui.setRenameWait(new Promise((resolve) => { release = resolve; }));
  ui.setRenameError('保存失败，请检查磁盘空间');
  ui.submit(); ui.submit(); await ui.settle();
  assert.equal(ui.calls.filter((c) => c.command === 'rename_track').length, 1);
  assert.equal(ui.field.disabled, true);
  ui.close.handlers.click(); assert.equal(ui.dialog.open, true);
  let prevented = false; ui.dialog.handlers.cancel({ preventDefault() { prevented = true; } });
  assert.equal(prevented, true);
  release(); await ui.settle();
  assert.equal(ui.dialog.open, true);
  assert.equal(ui.field.value, '新曲名');
  assert.equal(ui.field.disabled, false);
  assert.match(ui.node('track-name-form-error').textContent, /保存失败/);
  assert.equal(ui.announcements.length, 0);
});
