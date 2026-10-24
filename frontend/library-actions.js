'use strict';

(() => {
  const ui = window.MusicUI;
  const forms = window.MusicFeatures;
  if (!ui || !forms) return;
  const $ = (id) => document.getElementById(id);
  const dialog = $('remove-track-dialog');
  let active = null;
  let removed = false;

  $('remove-track-cancel').addEventListener('click', () => {
    if (dialog.dataset.busy !== 'true') dialog.close();
  });
  dialog.addEventListener('close', () => {
    if (!active) return;
    const buttons = [...document.querySelectorAll('.track-remove')];
    const target = removed ? buttons[Math.min(active.index, buttons.length - 1)]
      : buttons.find((button) => button.dataset.trackId === active.id);
    (target || $('library-title')).focus({ preventScroll: true });
    active = null;
  });
  forms.onSubmit('remove-track-form', (form) => {
    if (!active) return;
    const track = active;
    forms.featureAction(form, '移除中…', async () => {
      const result = await ui.mutate('remove_track', { trackId: track.id }, '', { scopedError: true });
      if (!result) throw new Error('客户端暂未就绪，请稍后重试。');
      removed = true;
      dialog.close();
      ui.announce(`已将“${track.title}”移出曲库，音频文件已保留`);
    }).then(() => {
      if (dialog.open && $('remove-track-form-error').textContent) $('remove-track-cancel').focus();
    });
  });

  window.MusicLibraryActions = {
    openRemove(track, index) {
      if (dialog.open) return;
      active = { id: track.id, title: track.title || track.filename, index };
      removed = false;
      $('remove-track-name').textContent = active.title;
      forms.errorAt('remove-track-form-error', null);
      dialog.showModal();
      $('remove-track-cancel').focus();
    },
  };
})();
