'use strict';

(() => {
  const ui = window.MusicUI;
  const forms = window.MusicFeatures;
  if (!ui || !forms) return;
  const $ = (id) => document.getElementById(id);
  const dialog = $('track-name-dialog');
  const field = $('track-name');
  const drafts = new Map();
  let active = null;
  let saved = false;

  function voicePreview() {
    const name = field.value.trim();
    $('track-name-voice').textContent = name
      ? `保存后可说：“小爱同学，播放本地 ${name}”。请先在“小爱语音”中开始接收指令。`
      : '填写曲名后，即可按这个名称语音点播。';
  }
  field.addEventListener('input', () => {
    forms.errorAt('track-name-form-error', null, 'track-name');
    voicePreview();
  });
  field.addEventListener('compositionend', voicePreview);
  $('track-name-cancel').addEventListener('click', () => {
    if (dialog.dataset.busy !== 'true') dialog.close();
  });
  dialog.addEventListener('close', () => {
    if (!active) return;
    if (saved) drafts.delete(active.id);
    else drafts.set(active.id, { value: field.value, base: active.title });
    // The owning row may have been replaced by a poll or removed by the filter.
    const trigger = [...document.querySelectorAll('.track-edit')].find((button) => button.dataset.trackId === active.id);
    (trigger || $('library-title')).focus({ preventScroll: true });
    active = null;
  });

  $('recognize-track-name').addEventListener('click', () => {
    if (!active) return;
    const trackId = active.id;
    forms.featureAction($('track-name-form'), '识别中…', async () => {
      $('track-name-recognition').textContent = '正在读取曲名信息…';
      try {
        const result = await ui.invoke('recognize_track_name', { trackId });
        if (result.title) {
          field.value = result.title;
          const source = { audio_tag: '音频标签', video_title: '视频标题', filename: '文件名' }[result.source] || '本地信息';
          $('track-name-recognition').textContent = `已从${source}提取。请核对曲名，保存后生效。`;
          voicePreview();
        } else {
          $('track-name-recognition').textContent = '未找到可靠曲名，请手动输入。';
        }
      } catch (error) {
        $('track-name-recognition').textContent = '识别未完成，可以重试或直接修改曲名。';
        throw error;
      }
    }, 'track-name', $('recognize-track-name'));
  });

  forms.onSubmit('track-name-form', (form) => {
    if (!active) return;
    const name = field.value.trim();
    if (!/[\p{L}\p{N}]/u.test(name) || [...name].length > 120 || /[\u0000-\u001f\u007f-\u009f]/u.test(name)) {
      forms.errorAt('track-name-form-error', '请输入 1–120 个字符的曲名，包含文字或数字，不含换行。', 'track-name');
      return;
    }
    const track = active;
    forms.featureAction(form, '保存中…', async () => {
      const result = await ui.mutate('rename_track', { trackId: track.id, title: name, expectedTitle: track.title }, '', { scopedError: true });
      if (!result) throw new Error('客户端暂未就绪，请稍后重试。');
      saved = true;
      dialog.close();
      ui.announce(`已保存曲名“${name}”，可说“播放本地 ${name}”`);
    }, 'track-name');
  });

  window.MusicTrackEditor = {
    open(track) {
      if (dialog.open) return;
      active = { id: track.id, title: track.title };
      saved = false;
      const draft = drafts.get(track.id);
      field.value = draft?.base === track.title ? draft.value : track.title;
      forms.errorAt('track-name-form-error', null, 'track-name');
      $('track-name-file').textContent = `原文件：${track.filename}`;
      $('track-name-recognition').textContent = '从音频标签、文件名或视频标题提取，不上传音频。';
      voicePreview();
      dialog.showModal();
      field.focus();
      field.select();
    },
  };
})();
