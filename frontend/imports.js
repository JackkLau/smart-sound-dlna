'use strict';

(() => {
  if (!window.MusicUI || !window.MusicFeatures) return;
  const { invoke, announce, element, formatTime } = window.MusicUI;
  const { featureAction, onSubmit, errorAt } = window.MusicFeatures;
  const $ = (id) => document.getElementById(id);
  const stores = { video: new BilibiliSelection(), favorite: new BilibiliSelection() };
  let source = 'video';
  let page = null;
  let busy = false;
  const current = () => stores[source];

  function syncSelection() {
    document.querySelectorAll('[data-part-cid]').forEach((input) => {
      const record = stores[input.dataset.source].records.get(input.dataset.bvid);
      input.checked = record?.selected.has(Number(input.dataset.partCid)) || false;
    });
    document.querySelectorAll('[data-all-parts]').forEach((input) => {
      const record = stores[input.dataset.source].records.get(input.dataset.bvid);
      input.checked = Boolean(record?.selected.size && record.selected.size === record.preview.parts.length);
      input.indeterminate = Boolean(record?.selected.size && !input.checked);
    });
    $('selection-count').textContent = `已选 ${current().count} 首${source === 'favorite' ? ' · 包含其他页的选择' : ''}`;
    $('download-form').hidden = !current().records.size;
    $('download-selected').disabled = busy || !current().count;
    $('clear-selection').disabled = busy || !current().count;
    $('favorite-previous').disabled = busy || !page || page.page <= 1;
    $('favorite-next').disabled = busy || !page?.has_more;
  }
  // One foreground import operation at a time prevents page, source and selection
  // changes from racing a native request. Background downloads remain independent.
  async function run(form, label, operation, field, trigger) {
    if (busy || $('bilibili-dialog').dataset.busy === 'true') return;
    busy = true;
    const controls = [...$('import-area').querySelectorAll('button,input,select')];
    const previous = controls.map((node) => node.disabled);
    const oldLabel = trigger?.textContent;
    controls.forEach((node) => { node.disabled = true; });
    if (trigger) { trigger.textContent = label; trigger.setAttribute('aria-busy', 'true'); }
    try { return await featureAction(form, label, operation, field); }
    finally {
      controls.forEach((node, i) => { node.disabled = previous[i]; });
      if (trigger) { trigger.textContent = oldLabel; trigger.setAttribute('aria-busy', 'false'); }
      busy = false;
      syncSelection();
      if (field && $(`${form.id}-error`).textContent) $(field).focus();
    }
  }
  function clearReport() {
    $('batch-summary').textContent = '';
    $('batch-results').replaceChildren();
    errorAt('download-form-error', null);
  }
  function changeSelection(operation) {
    if (busy) { syncSelection(); return; }
    try { operation(); errorAt('download-form-error', null); }
    catch (error) { errorAt('download-form-error', error); }
    syncSelection();
  }
  function picker(preview, mode) {
    const store = stores[mode];
    const fieldset = element('fieldset', 'part-picker');
    fieldset.append(element('legend', 'field-label', '选择要保存的音轨'));
    const allLabel = element('label', 'part-choice part-all');
    const all = element('input'); all.type = 'checkbox';
    all.dataset.allParts = 'true'; all.dataset.source = mode; all.dataset.bvid = preview.bvid;
    all.addEventListener('change', () => changeSelection(() => store.setAll(preview.bvid, all.checked)));
    allLabel.append(all, element('span', '', `全选此视频（${preview.parts.length} P）`));
    fieldset.append(allLabel);
    const list = element('div', 'part-list');
    for (const part of preview.parts) {
      const label = element('label', 'part-choice');
      const input = element('input'); input.type = 'checkbox';
      input.dataset.partCid = String(part.cid); input.dataset.source = mode; input.dataset.bvid = preview.bvid;
      input.addEventListener('change', () => changeSelection(() => store.set(preview.bvid, part.cid, input.checked)));
      label.append(input, element('span', '', `P${part.page} · ${part.title} · ${formatTime(part.duration)}`));
      list.append(label);
    }
    fieldset.append(list);
    return fieldset;
  }
  document.querySelectorAll('input[name=import-source]').forEach((input) => input.addEventListener('change', () => {
    if (busy || !input.checked) return;
    source = input.value;
    $('video-source').hidden = source !== 'video';
    $('favorite-source').hidden = source !== 'favorite';
    clearReport(); syncSelection();
    $(source === 'video' ? 'bilibili-url' : 'favorite-url').focus();
  }));
  onSubmit('bilibili-form', async (form) => {
    if (busy) return;
    const url = $('bilibili-url').value.trim();
    if (!url) { errorAt('bilibili-form-error', '请先粘贴视频链接或 BV 号。', 'bilibili-url'); return; }
    await run(form, '正在解析…', async () => {
      stores.video.clear(); $('video-preview').hidden = true; clearReport(); syncSelection();
      const preview = await invoke('parse_bilibili', { url });
      stores.video.register(preview, true);
      $('video-preview').replaceChildren(
        element('h3', '', preview.title),
        element('p', 'field-help', `${preview.author} · ${preview.parts.length} 个分 P · ${preview.bvid}`),
        picker(preview, 'video'),
      );
      $('video-preview').hidden = false;
      announce(`已解析 ${preview.parts.length} 个分 P`);
    }, 'bilibili-url');
  });
  async function loadFavorite(url, targetPage, form, field, trigger) {
    await run(form, '正在读取…', async () => {
      const result = await invoke('read_bilibili_favorite', { url, page: targetPage });
      if (page?.id !== result.id) { stores.favorite.clear(); clearReport(); }
      page = result;
      renderFavorite();
      announce(`已读取“${page.title}”第 ${page.page} 页，${page.videos.length} 个视频`);
    }, field, trigger);
  }
  onSubmit('favorite-form', async (form) => {
    const url = $('favorite-url').value.trim();
    if (!url) { errorAt('favorite-form-error', '请粘贴收藏夹链接或填写收藏夹 ID。', 'favorite-url'); return; }
    await loadFavorite(url, 1, form, 'favorite-url');
  });
  onSubmit('my-folders-form', async (form) => {
    await run(form, '正在读取…', async () => {
      $('my-folders-picker').hidden = true;
      $('my-folders-status').textContent = '';
      const folders = await invoke('list_bilibili_folders');
      $('my-folders').replaceChildren(...folders.map((folder) => {
        const option = element('option', '', `${folder.title} · ${folder.total} 个视频`);
        option.value = folder.id; return option;
      }));
      $('my-folders-picker').hidden = !folders.length;
      $('my-folders-status').textContent = folders.length ? `已读取 ${folders.length} 个收藏夹，请选择后打开。` : '此账号还没有创建收藏夹，也可以粘贴公开收藏夹链接。';
    });
  });
  $('favorite-open-folder').addEventListener('click', () => {
    const id = $('my-folders').value;
    if (id) loadFavorite(id, 1, $('my-folders-form'), 'my-folders', $('favorite-open-folder'));
  });
  for (const [id, step] of [['favorite-previous', -1], ['favorite-next', 1]]) {
    $(id).addEventListener('click', () => {
      if (page && !$(id).disabled) loadFavorite(page.id, page.page + step, $('favorite-preview-form'), undefined, $(id));
    });
  }
  onSubmit('favorite-preview-form', () => {});
  function renderFavorite() {
    $('favorite-preview-form').hidden = false;
    $('favorite-title').textContent = page.title;
    $('favorite-meta').textContent = `${page.owner} · 共 ${page.total} 个视频`;
    $('favorite-page').textContent = `第 ${page.page} 页 · 本页 ${page.videos.length} 个`;
    const rows = page.videos.map((video) => {
      const row = element('li', 'favorite-video');
      row.append(element('h4', '', video.title), element('p', 'field-help', `${video.author} · ${video.parts} P · ${formatTime(video.duration)}`));
      if (video.unavailable) { row.append(element('p', 'field-help', video.unavailable)); return row; }
      const button = element('button', 'text-button'); button.type = 'button';
      const content = element('div');
      function renderParts() {
        const record = stores.favorite.records.get(video.bvid);
        button.textContent = record ? '重新解析音轨' : `选择音轨（${video.parts} P）`;
        button.setAttribute('aria-label', `${button.textContent}：${video.title}`);
        if (record) content.replaceChildren(picker(record.preview, 'favorite'));
      }
      button.addEventListener('click', async () => {
        await run($('favorite-preview-form'), '正在解析…', async () => {
          const preview = await invoke('parse_bilibili', { url: video.bvid });
          stores.favorite.register(preview);
          renderParts();
        }, undefined, button);
        // run restores the pending trigger label; now reconcile it with the preview.
        renderParts(); syncSelection();
      });
      renderParts(); row.append(button, content); return row;
    });
    if (!rows.length) rows.push(element('li', 'favorite-empty', page.total ? '这一页已没有视频，收藏夹内容可能发生变化，请返回上一页或重新读取。' : '这个收藏夹还没有视频，可以换一个收藏夹。'));
    $('favorite-videos').replaceChildren(...rows);
  }
  $('clear-selection').addEventListener('click', () => changeSelection(() => { current().clearSelection(); clearReport(); }));
  onSubmit('download-form', async (form) => {
    await run(form, '正在加入队列…', async () => {
      if (!current().count) throw new Error('请先勾选要下载的音轨。');
      clearReport();
      const store = current();
      const result = await invoke('download_bilibili_batch', { selections: store.payload() });
      store.accept(result);
      const accepted = result.items.filter((item) => item.job).length;
      const skipped = result.items.filter((item) => !item.job);
      $('batch-summary').textContent = `已加入队列 ${accepted} 首${skipped.length ? `，跳过 ${skipped.length} 首（仍保留选择）` : ''}。下载进度显示在主界面的音乐库上方。`;
      $('batch-results').replaceChildren(...skipped.map((item) => element('li', '', `${item.title}：${item.error}`)));
      announce(`已加入下载队列 ${accepted} 首${skipped.length ? `，跳过 ${skipped.length} 首` : ''}`);
    });
  });
  syncSelection();
})();
