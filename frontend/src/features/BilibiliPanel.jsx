import React, { useEffect, useRef, useState } from 'react';
import { Dialog } from '../components.jsx';
import { formatTime } from '../utils.js';
import BilibiliSelection from '../../import-selection.js';
import { Form, ErrorMessage } from './Form.jsx';
import Account from './Account.jsx';

function PartPicker({ record, onChange, disabled }) {
  const all = useRef(null);
  const selected = record.selected;
  useEffect(() => {
    if (all.current)
      all.current.indeterminate = selected.size > 0 && selected.size < record.preview.parts.length;
  });
  return (
    <fieldset className="part-picker" disabled={disabled}>
      <legend className="field-label">选择要保存的音轨</legend>
      <label className="part-choice part-all">
        <input
          ref={all}
          type="checkbox"
          checked={selected.size === record.preview.parts.length && selected.size > 0}
          onChange={(event) => onChange('all', event.target.checked)}
        />
        <span>全选此视频（{record.preview.parts.length} P）</span>
      </label>
      <div className="part-list">
        {record.preview.parts.map((part) => (
          <label key={part.cid} className="part-choice">
            <input
              type="checkbox"
              checked={selected.has(part.cid)}
              onChange={(event) => onChange(part.cid, event.target.checked)}
            />
            <span>
              P{part.page} · {part.title} · {formatTime(part.duration)}
            </span>
          </label>
        ))}
      </div>
    </fieldset>
  );
}

export default function BilibiliPanel({ open, onClose, controller }) {
  const { features, disabled, pending, errors, run, invoke, announce, clearError } = controller;
  const stores = useRef({
    video: new BilibiliSelection(),
    favorite: new BilibiliSelection(),
  });
  const [source, setSource] = useState('video');
  const [url, setUrl] = useState('');
  const [favoriteUrl, setFavoriteUrl] = useState('');
  const [page, setPage] = useState(null);
  const [folders, setFolders] = useState(null);
  const [folderId, setFolderId] = useState('');
  const [report, setReport] = useState(null);
  const [, rerender] = useState(0);
  const urlField = useRef(null);
  const favoriteField = useRef(null);
  const current = stores.current[source];
  const updateSelection = (operation) => {
    if (disabled) return;
    run('selection', async () => {
      operation();
      clearError('download-form');
      rerender((value) => value + 1);
    });
  };
  const loadFavorite = (target, targetPage, key = 'favorite-form', field) =>
    run(
      key,
      async () => {
        const next = await invoke('read_bilibili_favorite', {
          url: target,
          page: targetPage,
        });
        if (page?.id !== next.id) {
          stores.current.favorite.clear();
          setReport(null);
        }
        setPage(next);
        announce(`已读取“${next.title}”第 ${next.page} 页，${next.videos.length} 个视频`);
      },
      field,
    );
  const picker = (record, mode) => (
    <PartPicker
      record={record}
      disabled={disabled}
      onChange={(cid, checked) =>
        updateSelection(() =>
          cid === 'all'
            ? stores.current[mode].setAll(record.preview.bvid, checked)
            : stores.current[mode].set(record.preview.bvid, cid, checked),
        )
      }
    />
  );
  const video = [...stores.current.video.records.values()][0];
  const selectionError = errors.selection || errors['download-form'];
  return (
    <Dialog
      id="bilibili-dialog"
      title="从 Bilibili 导入音频"
      eyebrow="扩充你的曲库"
      description="bilibili-description"
      open={open}
      onClose={onClose}
      busy={Boolean(pending)}
      className="feature-dialog"
    >
      <div className="dialog-body">
        <p id="bilibili-description" className="dialog-description">
          从视频或收藏夹选择音轨，保存为 M4A 并加入本地曲库。支持单 P 和多
          P；请仅下载你有权保存的内容。
        </p>
        <div id="import-area">
          <fieldset className="source-choice" disabled={disabled}>
            <legend className="field-label">导入来源</legend>
            {[
              ['video', '视频链接'],
              ['favorite', '收藏夹'],
            ].map(([value, label]) => (
              <label key={value}>
                <input
                  type="radio"
                  name="import-source"
                  value={value}
                  checked={source === value}
                  onChange={() => {
                    setSource(value);
                    setReport(null);
                    clearError('download-form');
                  }}
                />{' '}
                {label}
              </label>
            ))}
          </fieldset>
          <section id="video-source" aria-label="视频链接导入" hidden={source !== 'video'}>
            <Form
              noValidate
              id="bilibili-form"
              onSubmit={() =>
                run(
                  'bilibili-form',
                  async () => {
                    if (!url.trim()) throw new Error('请先粘贴视频链接或 BV 号。');
                    stores.current.video.clear();
                    setReport(null);
                    rerender((value) => value + 1);
                    const preview = await invoke('parse_bilibili', {
                      url: url.trim(),
                    });
                    stores.current.video.register(preview, true);
                    rerender((value) => value + 1);
                    announce(`已解析 ${preview.parts.length} 个分 P`);
                  },
                  urlField,
                )
              }
            >
              <label className="field-label" htmlFor="bilibili-url">
                视频链接或 BV 号
              </label>
              <div className="inline-field">
                <input
                  id="bilibili-url"
                  data-autofocus={source === 'video' ? true : undefined}
                  ref={urlField}
                  className="text-input"
                  autoComplete="off"
                  spellCheck={false}
                  placeholder="https://www.bilibili.com/video/BV…"
                  maxLength={2048}
                  disabled={disabled}
                  value={url}
                  onChange={(event) => setUrl(event.target.value)}
                  aria-invalid={Boolean(errors['bilibili-form'])}
                  aria-describedby="bilibili-help bilibili-form-error"
                />
                <button
                  className="button primary"
                  type="submit"
                  disabled={disabled}
                  aria-busy={pending === 'bilibili-form'}
                >
                  解析视频
                </button>
              </div>
              <p id="bilibili-help" className="field-help">
                支持普通视频、分 P 链接和 b23.tv 短链接。
              </p>
              <ErrorMessage id="bilibili-form-error">{errors['bilibili-form']}</ErrorMessage>
            </Form>
            {video && (
              <div id="video-preview" className="preview-panel">
                <h3>{video.preview.title}</h3>
                <p className="field-help">
                  {video.preview.author} · {video.preview.parts.length} 个分 P ·{' '}
                  {video.preview.bvid}
                </p>
                {picker(video, 'video')}
              </div>
            )}
          </section>
          <section id="favorite-source" aria-label="收藏夹导入" hidden={source !== 'favorite'}>
            <Form
              noValidate
              id="favorite-form"
              onSubmit={() => {
                if (!favoriteUrl.trim())
                  return run(
                    'favorite-form',
                    async () => {
                      throw new Error('请粘贴收藏夹链接或填写收藏夹 ID。');
                    },
                    favoriteField,
                  );
                return loadFavorite(favoriteUrl.trim(), 1, 'favorite-form', favoriteField);
              }}
            >
              <label className="field-label" htmlFor="favorite-url">
                收藏夹链接或 ID
              </label>
              <div className="inline-field">
                <input
                  id="favorite-url"
                  data-autofocus={source === 'favorite' ? true : undefined}
                  ref={favoriteField}
                  className="text-input"
                  autoComplete="off"
                  spellCheck={false}
                  maxLength={2048}
                  placeholder="https://space.bilibili.com/…/favlist?fid=…"
                  disabled={disabled}
                  value={favoriteUrl}
                  onChange={(event) => setFavoriteUrl(event.target.value)}
                  aria-invalid={Boolean(errors['favorite-form'])}
                  aria-describedby="favorite-help favorite-form-error"
                />
                <button
                  className="button primary"
                  type="submit"
                  disabled={disabled}
                  aria-busy={pending === 'favorite-form'}
                >
                  读取收藏夹
                </button>
              </div>
              <p id="favorite-help" className="field-help">
                公开收藏夹可直接读取；私密收藏夹需先登录有权限的账号。
              </p>
              <ErrorMessage id="favorite-form-error">{errors['favorite-form']}</ErrorMessage>
            </Form>
            <Form
              noValidate
              id="my-folders-form"
              onSubmit={() =>
                run('my-folders-form', async () => {
                  setFolders(null);
                  const next = await invoke('list_bilibili_folders');
                  setFolders(next);
                  setFolderId(next[0]?.id || '');
                })
              }
            >
              <button
                type="submit"
                className="button outline"
                disabled={disabled}
                aria-busy={pending === 'my-folders-form'}
              >
                读取我的收藏夹
              </button>
              {Boolean(folders?.length) && (
                <div id="my-folders-picker">
                  <label className="field-label" htmlFor="my-folders">
                    我的收藏夹
                  </label>
                  <div className="inline-field">
                    <select
                      id="my-folders"
                      className="text-input"
                      value={folderId}
                      onChange={(event) => setFolderId(event.target.value)}
                      disabled={disabled}
                    >
                      {folders.map((folder) => (
                        <option value={folder.id} key={folder.id}>
                          {folder.title} · {folder.total} 个视频
                        </option>
                      ))}
                    </select>
                    <button
                      id="favorite-open-folder"
                      type="button"
                      className="button outline"
                      disabled={disabled || !folderId}
                      onClick={() => loadFavorite(folderId, 1, 'my-folders-form')}
                    >
                      打开收藏夹
                    </button>
                  </div>
                </div>
              )}
              <p id="my-folders-status" className="field-help" role="status">
                {folders
                  ? folders.length
                    ? `已读取 ${folders.length} 个收藏夹，请选择后打开。`
                    : '此账号还没有创建收藏夹，也可以粘贴公开收藏夹链接。'
                  : ''}
              </p>
              <ErrorMessage id="my-folders-form-error">{errors['my-folders-form']}</ErrorMessage>
            </Form>
            {page && (
              <section id="favorite-preview-form" className="preview-panel" aria-label="收藏夹视频">
                <h3>{page.title}</h3>
                <p className="field-help">
                  {page.owner} · 共 {page.total} 个视频
                </p>
                <p className="field-help">
                  展开视频后勾选音轨；翻页保留已选内容，每批最多 100 首。
                </p>
                <ul className="favorite-videos">
                  {page.videos.map((item) => {
                    const record = stores.current.favorite.records.get(item.bvid);
                    return (
                      <li key={item.bvid} className="favorite-video">
                        <h4>{item.title}</h4>
                        <p className="field-help">
                          {item.author} · {item.parts} P · {formatTime(item.duration)}
                        </p>
                        {item.unavailable ? (
                          <p className="field-help">{item.unavailable}</p>
                        ) : (
                          <>
                            <button
                              type="button"
                              className="text-button"
                              disabled={disabled}
                              aria-label={`${record ? '重新解析音轨' : '选择音轨'}：${item.title}`}
                              onClick={() =>
                                run('favorite-preview-form', async () => {
                                  const preview = await invoke('parse_bilibili', {
                                    url: item.bvid,
                                  });
                                  stores.current.favorite.register(preview);
                                  rerender((value) => value + 1);
                                })
                              }
                            >
                              {record ? '重新解析音轨' : `选择音轨（${item.parts} P）`}
                            </button>
                            {record && picker(record, 'favorite')}
                          </>
                        )}
                      </li>
                    );
                  })}
                  {!page.videos.length && (
                    <li className="favorite-empty">
                      {page.total
                        ? '这一页已没有视频，收藏夹内容可能发生变化，请返回上一页或重新读取。'
                        : '这个收藏夹还没有视频，可以换一个收藏夹。'}
                    </li>
                  )}
                </ul>
                <ErrorMessage id="favorite-preview-form-error">
                  {errors['favorite-preview-form']}
                </ErrorMessage>
                <div className="favorite-pagination">
                  <button
                    type="button"
                    className="button outline"
                    disabled={disabled || page.page <= 1}
                    onClick={() => loadFavorite(page.id, page.page - 1, 'favorite-preview-form')}
                  >
                    上一页
                  </button>
                  <span role="status">
                    第 {page.page} 页 · 本页 {page.videos.length} 个
                  </span>
                  <button
                    type="button"
                    className="button outline"
                    disabled={disabled || !page.has_more}
                    onClick={() => loadFavorite(page.id, page.page + 1, 'favorite-preview-form')}
                  >
                    下一页
                  </button>
                </div>
              </section>
            )}
          </section>
          {current.records.size > 0 && (
            <Form
              noValidate
              id="download-form"
              className="preview-panel"
              onSubmit={() =>
                run('download-form', async () => {
                  if (!current.count) throw new Error('请先勾选要下载的音轨。');
                  setReport(null);
                  const response = await invoke('download_bilibili_batch', {
                    selections: current.payload(),
                  });
                  current.accept(response);
                  setReport(response);
                  rerender((value) => value + 1);
                  const accepted = response.items.filter((item) => item.job).length;
                  const skipped = response.items.length - accepted;
                  announce(`已加入下载队列 ${accepted} 首${skipped ? `，跳过 ${skipped} 首` : ''}`);
                })
              }
            >
              <div className="selection-summary">
                <strong id="selection-count" role="status">
                  已选 {current.count} 首{source === 'favorite' ? ' · 包含其他页的选择' : ''}
                </strong>
                <button
                  type="button"
                  className="text-button"
                  disabled={disabled || !current.count}
                  onClick={() =>
                    updateSelection(() => {
                      current.clearSelection();
                      setReport(null);
                    })
                  }
                >
                  清空选择
                </button>
              </div>
              <p className="field-help">
                保留原始 AAC 音轨，不下载画面。已在曲库或队列中的音轨会跳过。
              </p>
              <ErrorMessage id="download-form-error">{selectionError}</ErrorMessage>
              {report && (
                <>
                  <p id="batch-summary" className="field-help" role="status">
                    已加入队列 {report.items.filter((item) => item.job).length} 首
                    {report.items.some((item) => !item.job)
                      ? `，跳过 ${report.items.filter((item) => !item.job).length} 首（仍保留选择）`
                      : ''}
                    。下载进度显示在页面下方的导入区。
                  </p>
                  <ul className="batch-results">
                    {report.items
                      .filter((item) => !item.job)
                      .map((item) => (
                        <li key={`${item.preview_id}-${item.cid}`}>
                          {item.title}：{item.error}
                        </li>
                      ))}
                  </ul>
                </>
              )}
              <div className="dialog-actions">
                <button
                  className="button primary"
                  type="submit"
                  disabled={disabled || !current.count}
                  aria-busy={pending === 'download-form'}
                >
                  下载选中音轨
                </button>
              </div>
            </Form>
          )}
        </div>
        <details className="feature-details">
          <summary>账号与保存位置</summary>
          <p className="field-help">
            公开视频可直接解析。需要登录的视频可在官方页面登录；登录状态由系统安全保存，重启后自动验证并恢复，退出登录会清除。
          </p>
          <Account provider="bilibili" controller={controller} />
          <p className="field-label">保存位置</p>
          <p className="field-help path-copy">{features.downloads?.directory}</p>
        </details>
        <p className="field-help dialog-footnote">
          可以关闭此窗口继续听歌。下载进度显示在页面下方的导入区；退出客户端会中断未完成的任务。
        </p>
      </div>
    </Dialog>
  );
}
