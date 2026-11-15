import { useCallback, useEffect, useRef, useState, type ChangeEvent, type FormEvent } from 'react';
import { ClearableInput } from '../components.jsx';
import type { Invoke } from './types';
import { videoMessage } from './useVideo';

const labels = {
  fetchingMetadata: '正在获取影片列表',
  downloading: '正在下载',
  complete: '下载完成',
  cancelled: '下载已取消',
  failed: '下载失败',
};

type Download = {
  id: string;
  status: keyof typeof labels;
  name: string;
  progressBytes: number;
  totalBytes: number;
  speedBytesPerSec: number;
  files: { index: number; name: string; size: number }[];
  error: string | null;
};

function downloadState(value: unknown): Download | null {
  if (value === null) return null;
  const task = value as Download;
  if (
    !task ||
    typeof task.id !== 'string' ||
    !Object.hasOwn(labels, task.status) ||
    !Array.isArray(task.files)
  ) {
    throw new Error('下载状态无法读取，请重新检查。');
  }
  return task;
}

function bytes(value: number): string {
  for (const [unit, divisor] of [
    ['GiB', 1024 ** 3],
    ['MiB', 1024 ** 2],
    ['KiB', 1024],
  ] as const) {
    if (value >= divisor) return (value / divisor).toFixed(1) + ' ' + unit;
  }
  return Math.max(0, Math.floor(value)) + ' B';
}

export default function MagnetDownload({
  invoke,
  visible,
  ready,
  canOpen,
  openFile,
}: {
  invoke: Invoke;
  visible: boolean;
  ready: boolean;
  canOpen: boolean;
  openFile: (id: string, index: number) => Promise<void>;
}) {
  const [task, setTask] = useState<Download | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [link, setLink] = useState('');
  const [fieldError, setFieldError] = useState('');
  const [actionError, setActionError] = useState('');
  const [readError, setReadError] = useState('');
  const [pending, setPending] = useState<'start' | 'cancel' | null>(null);
  const input = useRef<HTMLInputElement>(null);
  const control = useRef({ mounted: true, changing: false, generation: 0, reading: false });

  const read = useCallback(async () => {
    const current = control.current;
    if (current.reading || current.changing) return;
    const generation = current.generation;
    current.reading = true;
    try {
      const next = downloadState(await invoke('video_magnet_status'));
      if (!current.mounted || generation !== current.generation) return;
      setTask(next);
      setLoaded(true);
      setReadError('');
    } catch (failure) {
      if (current.mounted && generation === current.generation) setReadError(videoMessage(failure));
    } finally {
      current.reading = false;
    }
  }, [invoke]);

  useEffect(() => {
    control.current.mounted = true;
    return () => {
      control.current.mounted = false;
    };
  }, []);

  useEffect(() => {
    if (!visible || !ready) return;
    void read();
    const timer = setInterval(() => void read(), 2000);
    return () => {
      clearInterval(timer);
      control.current.generation++;
    };
  }, [visible, ready, read]);

  const active = task?.status === 'fetchingMetadata' || task?.status === 'downloading';
  const busy = pending !== null || !ready || !loaded;

  async function change(action: 'start' | 'cancel') {
    const current = control.current;
    if (busy || current.changing) return;
    current.changing = true;
    current.generation++;
    setPending(action);
    setActionError('');
    try {
      const next = downloadState(
        await (action === 'start'
          ? invoke('video_magnet_start', { magnet: link.trim() })
          : invoke('video_magnet_cancel', { id: task?.id })),
      );
      if (!current.mounted) return;
      // Dismissing the native folder picker returns the previous task.
      setTask(next);
      setReadError('');
    } catch (failure) {
      if (current.mounted) setActionError(videoMessage(failure));
    } finally {
      current.changing = false;
      if (current.mounted) setPending(null);
    }
  }

  function submit(event: FormEvent) {
    event.preventDefault();
    if (busy || active) return;
    if (!/^magnet:\?[^\s]+$/i.test(link.trim())) {
      setFieldError('请粘贴以 magnet:? 开头的完整链接。');
      input.current?.focus();
      return;
    }
    void change('start');
  }

  const updateLink = (value: string) => {
    setLink(value);
    setFieldError('');
    setActionError('');
  };

  return (
    <section className="video-evidence video-magnet" aria-labelledby="video-magnet-heading">
      <h2 id="video-magnet-heading">磁力下载</h2>
      {!active && (
        <form noValidate onSubmit={submit}>
          <label className="field-label" htmlFor="video-magnet-link">
            影片磁力链接
          </label>
          <div className="magnet-input-row">
            <ClearableInput
              id="video-magnet-link"
              type="text"
              inputMode="url"
              className="text-input"
              placeholder="magnet:?xt=urn:btih:…"
              autoComplete="off"
              spellCheck={false}
              inputRef={input}
              value={link}
              disabled={busy}
              aria-invalid={Boolean(fieldError)}
              aria-describedby={fieldError ? 'magnet-field-error' : 'magnet-help'}
              onClear={() => updateLink('')}
              onChange={(event: ChangeEvent<HTMLInputElement>) => updateLink(event.target.value)}
            />
            <button
              type="submit"
              className="button outline"
              disabled={busy || !link.trim()}
              aria-busy={pending === 'start'}
            >
              {pending === 'start' ? '正在选择…' : '选择目录并下载'}
            </button>
          </div>
          {fieldError && (
            <p className="field-error" role="alert" id="magnet-field-error">
              {fieldError}
            </p>
          )}
          <p className="field-help" id="magnet-help">
            下载时请保持应用运行。完成后可选择影片投送。
          </p>
        </form>
      )}
      {readError && (
        <p className="field-error" role="alert">
          {readError}{' '}
          <button
            type="button"
            className="text-button"
            disabled={!ready || pending !== null}
            onClick={() => void read()}
          >
            重新读取下载状态
          </button>
        </p>
      )}
      {actionError && (
        <p className="field-error" role="alert">
          {actionError}
        </p>
      )}
      {task && (
        <div className="magnet-task">
          <div className="magnet-task-heading">
            <div>
              <strong role="status">{labels[task.status]}</strong>
              {task.name && (
                <p className="magnet-name" dir="auto">
                  {task.name}
                </p>
              )}
            </div>
            {active && (
              <button
                className="text-button"
                type="button"
                disabled={busy}
                onClick={() => void change('cancel')}
              >
                {pending === 'cancel' ? '正在取消…' : '取消下载'}
              </button>
            )}
          </div>
          {task.status === 'fetchingMetadata' && (
            <p className="field-help">正在连接提供种子的节点，等待对方发送文件信息。</p>
          )}
          {task.status === 'downloading' && (
            <>
              <progress
                aria-label="影片下载进度"
                max={task.totalBytes || 1}
                value={task.totalBytes ? task.progressBytes : undefined}
              />
              <p className="magnet-stats">
                {bytes(task.progressBytes)} / {bytes(task.totalBytes)} ·{' '}
                {bytes(task.speedBytesPerSec)}/s
              </p>
            </>
          )}
          {task.status === 'failed' && task.error && (
            <p className="field-error" role="alert">
              {task.error}
            </p>
          )}
          {task.status === 'complete' && (
            <ul className="magnet-files" aria-label="已下载影片">
              {task.files.map((file) => (
                <li key={file.index}>
                  <span className="magnet-file">
                    <strong dir="auto">{file.name}</strong>
                    <small>{bytes(file.size)}</small>
                  </span>
                  <button
                    type="button"
                    className="button outline"
                    aria-label={'载入影片 ' + file.name}
                    disabled={!canOpen || busy}
                    onClick={() => void openFile(task.id, file.index)}
                  >
                    载入影片
                  </button>
                </li>
              ))}
            </ul>
          )}
        </div>
      )}
    </section>
  );
}
