import { useEffect, useRef, useState } from 'react';
import type { Invoke } from './types';
import { videoMessage } from './useVideo';

type Progress = {
  processed: number;
  total: number;
  pending_review: number;
  failed: number;
  done: boolean;
  message: string;
};

export default function MovieBatch({
  invoke,
  revision,
  onRefresh,
}: {
  invoke: Invoke;
  revision: number;
  onRefresh: () => void;
}) {
  const [state, setState] = useState<Progress | null>(null);
  const [running, setRunning] = useState(false);
  const [pausing, setPausing] = useState(false);
  const [error, setError] = useState('');
  const owner = useRef({ mounted: false, working: false, paused: false, epoch: 0 });
  useEffect(() => {
    const control = owner.current;
    control.mounted = true;
    return () => {
      control.mounted = false;
      control.paused = true;
      control.epoch++;
    };
  }, []);
  useEffect(() => {
    const control = owner.current;
    const epoch = control.epoch;
    let active = true;
    if (!control.working) {
      invoke('movie_library_batch_status')
        .then((value) => {
          if (active && epoch === control.epoch) setState(value as Progress);
        })
        .catch((failure) => {
          if (active && epoch === control.epoch) setError(videoMessage(failure));
        });
    }
    return () => {
      active = false;
    };
  }, [invoke, revision]);
  async function run() {
    const control = owner.current;
    if (control.working) return;
    control.working = true;
    control.paused = false;
    control.epoch++;
    setRunning(true);
    setPausing(false);
    setError('');
    let processed = state?.processed;
    try {
      do {
        const next = (await invoke('movie_library_batch_advance')) as Progress;
        if (!control.mounted) return;
        setState(next);
        if (next.done || processed === next.processed) break;
        processed = next.processed;
      } while (!control.paused);
    } catch (failure) {
      if (control.mounted) setError(videoMessage(failure));
    } finally {
      control.working = false;
      if (control.mounted) {
        setRunning(false);
        setPausing(false);
        onRefresh();
      }
    }
  }
  return (
    <section className="movie-batch" aria-label="批量查找资源">
      <div className="movie-actions">
        {running ? (
          <button
            type="button"
            className="text-button"
            disabled={pausing}
            onClick={() => {
              owner.current.paused = true;
              setPausing(true);
            }}
          >
            {pausing ? '完成当前影片后暂停…' : '暂停查找'}
          </button>
        ) : (
          <button
            type="button"
            className="text-button"
            disabled={state?.total === 0 || (state?.done && !state.failed)}
            onClick={() => void run()}
          >
            {state?.done && state.failed
              ? '重试失败项'
              : state?.processed && !state.done
                ? '继续批量查找'
                : '批量查找资源'}
          </button>
        )}
        {state && (
          <span className="field-help">
            已检查 {state.processed} / {state.total} 部 · 待确认 {state.pending_review} 项 · 失败{' '}
            {state.failed} 部
          </span>
        )}
      </div>
      {state?.message && (
        <p className="field-help" role="status">
          {state.message}
        </p>
      )}
      {error && (
        <p className="field-error" role="alert">
          {error}
        </p>
      )}
    </section>
  );
}
