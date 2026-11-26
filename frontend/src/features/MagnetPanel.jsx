import React, { useRef, useState } from 'react';
import { ClearableInput, Dialog } from '../components.jsx';
import { formatSize } from '../utils.js';
import useMagnetClipboard from './useMagnetClipboard.js';
import { ErrorMessage, Form } from './Form.jsx';

const phases = {
  resolving: '正在获取种子信息',
  downloading: '正在下载',
  queued: '等待下载',
  paused: '已暂停',
  stopped: '已停止',
  failed: '下载失败',
  completed: '下载完成',
};

export default function MagnetPanel({ open, onClose, controller }) {
  const { features, disabled, pending, errors, run, invoke, announce, clearError } = controller;
  const field = useRef(null);
  const clipboard = useMagnetClipboard({ open, invoke, field, clearError, disabled });
  const { uri } = clipboard;
  const magnets = features.magnets;
  return (
    <Dialog
      open={open}
      onClose={onClose}
      busy={pending === 'magnet-form'}
      title="磁力链接下载"
      id="magnet-dialog"
    >
      <div className="dialog-body">
        <p className="field-help">打开此窗口或切回客户端时，自动识别剪贴板里的磁力链接。</p>
        {magnets?.error && (
          <p className="notice warning" role="status">
            {magnets.error}
          </p>
        )}
        {magnets && !magnets.available && !magnets.error && (
          <p className="notice warning" role="status">
            尚未找到 aria2。macOS 可运行 brew install aria2；Windows 请将 aria2c.exe
            放到客户端同目录， 或安装到 PATH。也可通过 TV_DLNA_ARIA2C
            指定程序完整路径，然后重新打开客户端。
          </p>
        )}
        <Form
          noValidate
          id="magnet-form"
          onSubmit={() =>
            run(
              'magnet-form',
              async () => {
                let valid = false;
                try {
                  const parsed = new URL(uri.trim());
                  valid =
                    parsed.protocol === 'magnet:' &&
                    parsed.searchParams
                      .getAll('xt')
                      .some((value) => /^urn:btih:(?:[a-f\d]{40}|[a-z2-7]{32})$/i.test(value));
                } catch {
                  /* The linked field error supplies a correction hint. */
                }
                if (!valid) throw new Error('请粘贴包含有效 BTIH 的 magnet:? 链接。');
                await invoke('add_magnet', { uri: uri.trim() });
                clipboard.submitted(uri);
                announce('已添加磁力下载任务');
              },
              field,
            )
          }
        >
          <label className="field-label" htmlFor="magnet-uri">
            磁力链接
          </label>
          <ClearableInput
            id="magnet-uri"
            inputRef={field}
            data-autofocus
            className="text-input"
            value={uri}
            disabled={disabled}
            placeholder="magnet:?xt=urn:btih:…"
            maxLength={8192}
            autoComplete="off"
            spellCheck={false}
            aria-invalid={Boolean(errors['magnet-form'])}
            aria-describedby="magnet-help magnet-form-error"
            onChange={(event) => clipboard.edit(event.target.value)}
            onClear={() => clipboard.edit('')}
            onPaste={clipboard.onPaste}
          />
          <p id="magnet-help" className="field-help">
            支持 BitTorrent v1 和混合种子。获取种子信息时需要连接到可用节点。
          </p>
          <button
            type="button"
            className="text-button"
            disabled={disabled}
            onClick={() => clipboard.read()}
          >
            重新读取剪贴板
          </button>
          <p className="field-help" role="status">
            {clipboard.notice}
          </p>
          {clipboard.candidates.some((item) => item.uri !== uri) && (
            <ul className="magnet-candidates" aria-label="识别到的磁力链接">
              {clipboard.candidates.map((item) => (
                <li key={item.id}>
                  <button
                    type="button"
                    className="text-button"
                    disabled={disabled || item.uri === uri}
                    onClick={() => clipboard.choose(item)}
                  >
                    {item.title} · {item.id.slice(0, 8)}
                  </button>
                </li>
              ))}
            </ul>
          )}
          <ErrorMessage id="magnet-form-error">{errors['magnet-form']}</ErrorMessage>
          <button
            type="submit"
            className="button primary"
            disabled={disabled || !magnets?.available}
            aria-busy={pending === 'magnet-form'}
          >
            开始下载
          </button>
        </Form>
        <p className="field-help magnet-directory">保存位置：{magnets?.directory || '正在读取…'}</p>
        <p className="field-help dialog-footnote">
          下载进度显示在导入区。关闭此窗口后继续下载；退出客户端会中断任务，重启后点击继续可续传。
          下载期间会向其他节点上传数据，完成后停止做种。下载的音视频可通过本地导入功能添加。
        </p>
      </div>
    </Dialog>
  );
}

export function MagnetQueue({ controller }) {
  const { features, disabled, pending, errors, run, invoke, announce } = controller;
  const magnets = features.magnets;
  const [viewPage, setViewPage] = useState(1);
  const [clearOpen, setClearOpen] = useState(false);
  const jobs = magnets?.jobs || [];
  const pages = Math.max(1, Math.ceil(jobs.length / 10));
  const page = Math.min(viewPage, pages);
  if (!jobs.length) return null;
  const action = (job, operation) =>
    run(`magnet-${job.id}`, async () => {
      await invoke('control_magnet', { id: job.id, action: operation });
      announce(
        `${operation === 'pause' ? '已暂停' : operation === 'stop' ? '已停止' : '已继续'}：${job.title}`,
      );
    });
  return (
    <section className="download-panel magnet-queue" aria-labelledby="magnet-queue-title">
      <div className="magnet-queue-heading">
        <h3 id="magnet-queue-title">磁力下载 · {jobs.length} 个任务</h3>
        <button
          type="button"
          className="text-button"
          disabled={disabled}
          onClick={() => run('magnet-open', () => invoke('open_magnet_directory'))}
        >
          打开下载目录
        </button>
      </div>
      <p className="field-help">暂停和停止均保留文件。下载完成后，可手动导入音视频。</p>
      {!magnets.available && (
        <p className="field-help">未找到 aria2，打开“磁力链接下载”可查看安装方法。</p>
      )}
      <ul className="magnet-jobs">
        {[...jobs]
          .reverse()
          .slice((page - 1) * 10, page * 10)
          .map((job) => (
            <li key={job.id} className="download-job">
              <div>
                <strong>{job.title}</strong>
                <p>
                  {phases[job.phase] || '正在读取状态'} · {formatSize(job.downloaded)}
                  {job.total > 0 && ` / ${formatSize(job.total)}`}
                  {job.phase === 'downloading' &&
                    ` · ${formatSize(job.speed)}/秒 · ${job.connections} 个连接`}
                </p>
                {['resolving', 'downloading', 'queued'].includes(job.phase) && (
                  <progress
                    aria-label={`${job.title}：${phases[job.phase]}`}
                    {...(job.phase === 'downloading' && job.total > 0
                      ? { max: job.total, value: Math.min(job.downloaded, job.total) }
                      : {})}
                  />
                )}
                <p className="job-error" role="status">
                  {job.error}
                </p>
              </div>
              {job.phase !== 'completed' && (
                <div className="magnet-job-actions" aria-busy={pending === `magnet-${job.id}`}>
                  {['paused', 'stopped', 'failed'].includes(job.phase) ? (
                    <button
                      type="button"
                      className="text-button"
                      disabled={disabled || !magnets.available}
                      aria-label={`${job.phase === 'failed' ? '重试' : '继续'} ${job.title}`}
                      onClick={() => action(job, 'resume')}
                    >
                      {job.phase === 'failed' ? '重试' : '继续'}
                    </button>
                  ) : (
                    <button
                      type="button"
                      className="text-button"
                      disabled={disabled}
                      aria-label={`暂停 ${job.title}`}
                      onClick={() => action(job, 'pause')}
                    >
                      暂停
                    </button>
                  )}
                  {job.phase !== 'stopped' && (
                    <button
                      type="button"
                      className="text-button"
                      disabled={disabled}
                      aria-label={`停止 ${job.title}`}
                      onClick={() => action(job, 'stop')}
                    >
                      停止
                    </button>
                  )}
                </div>
              )}
            </li>
          ))}
      </ul>
      <div className="magnet-queue-heading">
        <nav className="pagination" aria-label="磁力下载分页">
          <button
            type="button"
            className="text-button"
            disabled={page <= 1}
            onClick={() => setViewPage(page - 1)}
          >
            上一页
          </button>
          <span>
            {page} / {pages}
          </span>
          <button
            type="button"
            className="text-button"
            disabled={page >= pages}
            onClick={() => setViewPage(page + 1)}
          >
            下一页
          </button>
        </nav>
        <button
          type="button"
          className="text-button"
          disabled={disabled || !jobs.some((job) => ['completed', 'stopped'].includes(job.phase))}
          onClick={() => setClearOpen(true)}
        >
          清除结束记录（保留文件）
        </button>
      </div>
      <ErrorMessage id="magnet-queue-error">
        {magnets.error ||
          errors.poll ||
          Object.entries(errors)
            .filter(([key]) => key.startsWith('magnet-') && key !== 'magnet-form')
            .map(([, value]) => value)
            .filter(Boolean)
            .join('。')}
      </ErrorMessage>
      <Dialog
        id="magnet-clear-dialog"
        title="清除结束记录"
        description="magnet-clear-description"
        open={clearOpen}
        busy={pending === 'magnet-clear'}
        onClose={() => setClearOpen(false)}
      >
        <div className="dialog-body">
          <p id="magnet-clear-description">
            清除已完成和已停止任务的记录？下载文件会保留，暂停和失败的任务不受影响。
          </p>
          <ErrorMessage id="magnet-clear-error">{errors['magnet-clear']}</ErrorMessage>
          <div className="dialog-actions">
            <button
              type="button"
              className="button outline"
              data-autofocus
              disabled={disabled}
              onClick={() => setClearOpen(false)}
            >
              取消
            </button>
            <button
              type="button"
              className="button primary"
              disabled={disabled}
              aria-busy={pending === 'magnet-clear'}
              onClick={() =>
                run('magnet-clear', async () => {
                  await invoke('clear_finished_magnets');
                  setClearOpen(false);
                  announce('已清除结束记录，下载文件已保留');
                })
              }
            >
              清除记录
            </button>
          </div>
        </div>
      </Dialog>
    </section>
  );
}
