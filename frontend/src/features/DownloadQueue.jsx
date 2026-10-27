import React from 'react';
import { formatSize } from '../utils.js';
import { ErrorMessage } from './Form.jsx';

const phases = {
  queued: '等待下载',
  resolving: '正在获取音轨',
  downloading: '正在下载',
  cancelling: '正在取消',
  importing: '正在加入曲库',
  completed: '已加入曲库',
  failed: '下载未完成',
  cancelled: '已取消',
};

export default function DownloadQueue({ controller }) {
  const { features, disabled, pending, errors, run, invoke } = controller;
  const jobs = features.downloads?.jobs || [];
  if (!jobs.length) return null;
  return (
    <section id="download-panel" className="download-panel" aria-labelledby="download-title">
      <h3 id="download-title">Bilibili 下载进度</h3>
      <p className="field-help">关闭导入窗口后，电脑会继续下载。</p>
      <ul id="download-jobs">
        {[...jobs].reverse().map((job) => (
          <li key={job.id} className="download-job">
            <div>
              <strong>{job.title}</strong>
              <p>
                {phases[job.phase] || job.phase}
                {job.downloaded
                  ? ` · ${formatSize(job.downloaded)}${job.total ? ` / ${formatSize(job.total)}` : ''}`
                  : ''}
              </p>
              {['downloading', 'resolving', 'importing'].includes(job.phase) && (
                <progress
                  aria-label={`${job.title}：${phases[job.phase]}`}
                  {...(job.total && job.phase === 'downloading'
                    ? { max: job.total, value: job.downloaded }
                    : {})}
                />
              )}
              <p className="job-error" role="status">
                {job.error}
              </p>
            </div>
            {!['completed', 'importing'].includes(job.phase) && (
              <button
                type="button"
                className="text-button"
                disabled={disabled || job.phase === 'cancelling'}
                aria-label={`${['failed', 'cancelled'].includes(job.phase) ? '重试' : '取消'} ${job.title}`}
                aria-busy={pending === `job-${job.id}`}
                onClick={() =>
                  run(`job-${job.id}`, () =>
                    invoke(
                      ['failed', 'cancelled'].includes(job.phase)
                        ? 'retry_download'
                        : 'cancel_download',
                      { id: job.id },
                    ),
                  )
                }
              >
                {['failed', 'cancelled'].includes(job.phase) ? '重试' : '取消'}
              </button>
            )}
          </li>
        ))}
      </ul>
      <ErrorMessage id="download-error">
        {errors.poll ||
          Object.entries(errors)
            .filter(([key]) => key.startsWith('job-'))
            .map(([, value]) => value)
            .filter(Boolean)
            .join('。')}
      </ErrorMessage>
    </section>
  );
}
