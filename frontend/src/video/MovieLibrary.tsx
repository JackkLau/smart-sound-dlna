import { useEffect, useRef, useState } from 'react';
import { Dialog } from '../components.jsx';
import MovieBatch from './MovieBatch';
import type { Invoke } from './types';
import { videoMessage } from './useVideo';

import MovieDetail, { type Movie, type Start } from './MovieDetail';

const pageSize = 16;

export default function MovieLibrary({
  invoke,
  onClose,
  start,
  canStart,
}: {
  invoke: Invoke;
  onClose: () => void;
  start: Start;
  canStart: boolean;
}) {
  const [query, setQuery] = useState('');
  const [movies, setMovies] = useState<Movie[]>([]);
  const [selected, setSelected] = useState('');
  const [page, setPage] = useState(0);
  const [revision, setRevision] = useState(0);
  const [loading, setLoading] = useState(true);
  const [filter, setFilter] = useState('all');
  const [reviewIds, setReviewIds] = useState<Set<string>>(new Set());
  const [reviewError, setReviewError] = useState('');
  const [readError, setReadError] = useState('');
  const [actionError, setActionError] = useState('');
  const [feedback, setFeedback] = useState('');
  const [pending, setPending] = useState('');
  const lock = useRef(false);
  const pauseSync = useRef(false);
  const [pausing, setPausing] = useState(false);
  const alive = useRef(false);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
      pauseSync.current = true;
    };
  }, []);
  useEffect(() => {
    let active = true;
    setLoading(true);
    const timer = setTimeout(
      async () => {
        try {
          const list = (await invoke('movie_library_list', { query: query.trim() })) as Movie[];
          if (active) {
            setMovies(list);
            setReadError('');
          }
        } catch (failure) {
          if (active) {
            setMovies([]);
            setReadError(videoMessage(failure));
          }
        } finally {
          if (active) setLoading(false);
        }
      },
      query.trim() ? 140 : 0,
    );
    return () => {
      active = false;
      clearTimeout(timer);
    };
  }, [query, revision, invoke]);
  useEffect(() => {
    let active = true;
    invoke('movie_library_review_ids')
      .then((value) => {
        if (active) {
          setReviewIds(new Set(value as string[]));
          setReviewError('');
        }
      })
      .catch((failure) => {
        if (active) setReviewError(videoMessage(failure));
      });
    return () => {
      active = false;
    };
  }, [invoke, revision]);
  const rows = movies.filter(
    (item) =>
      filter === 'all' ||
      (filter === 'saved' && item.resource_count > 0) ||
      (filter === 'pending' && reviewIds.has(item.id)),
  );
  const lastPage = Math.max(0, Math.ceil(rows.length / pageSize) - 1);
  const currentPage = Math.min(page, lastPage);
  const current = rows.find((item) => item.id === selected);
  async function update(action: 'sync' | 'import') {
    if (lock.current) return;
    lock.current = true;
    setPending(action);
    setActionError('');
    setFeedback('');
    try {
      if (action === 'sync') {
        pauseSync.current = false;
        setPausing(false);
        let pages = 0;
        let changed = 0;
        do {
          const result = (await invoke('movie_library_sync')) as {
            changed: number;
            pages: number;
            complete: boolean;
            message: string;
          };
          if (!alive.current) return;
          pages += result.pages;
          changed += result.changed;
          setFeedback('本次检查 ' + pages + ' 页，更新 ' + changed + ' 项。' + result.message);
          setRevision((n) => n + 1);
          if (result.complete || result.pages === 0 || pauseSync.current) break;
          await new Promise((resolve) => setTimeout(resolve, 2400));
        } while (alive.current && !pauseSync.current);
        return;
      } else {
        const count = (await invoke('movie_library_import')) as number | null;
        if (!alive.current || count === null) return;
        setFeedback('片单处理完成，更新 ' + count + ' 项。');
      }
      setRevision((v) => v + 1);
    } catch (failure) {
      if (alive.current) setActionError(videoMessage(failure));
    } finally {
      lock.current = false;
      if (alive.current) {
        setPending('');
        setPausing(false);
      }
    }
  }
  return (
    <Dialog
      id="movie-library-dialog"
      title="电影资源库"
      eyebrow=""
      description="movie-library-criteria"
      open
      onClose={onClose}
      className="feature-dialog movie-library-dialog"
    >
      <div className="dialog-body">
        <p className="dialog-description" id="movie-library-criteria">
          收录 1990 年至今、豆瓣评分 8.0 分及以上的电影。
        </p>
        <div className="movie-toolbar">
          <label className="movie-query">
            <span className="field-label">搜索本地片库</span>
            <input
              className="text-input"
              type="search"
              data-autofocus
              value={query}
              placeholder="片名、原名、年份或豆瓣 ID"
              onChange={(e) => {
                setPage(0);
                setQuery(e.target.value);
              }}
            />
          </label>
          <div className="movie-actions">
            {pending === 'sync' ? (
              <button
                type="button"
                className="button outline"
                disabled={pausing}
                onClick={() => {
                  pauseSync.current = true;
                  setPausing(true);
                }}
              >
                {pausing ? '正在暂停…' : '暂停豆瓣更新'}
              </button>
            ) : (
              <button
                type="button"
                className="button outline"
                disabled={!!pending}
                onClick={() => void update('sync')}
              >
                更新豆瓣片库
              </button>
            )}
            <button
              type="button"
              className="text-button"
              title="选择本地 JSON 片单"
              disabled={!!pending}
              onClick={() => void update('import')}
            >
              导入片单
            </button>
          </div>
        </div>
        {feedback && (
          <p className="field-help" role="status">
            {feedback}
          </p>
        )}
        {actionError && (
          <p className="field-error" role="alert">
            {actionError}
          </p>
        )}
        <MovieBatch
          invoke={invoke}
          revision={revision}
          onRefresh={() => setRevision((n) => n + 1)}
        />
        <div className="movie-grid">
          <section className="movie-index" aria-label="片库列表">
            <div className="movie-list-head">
              <strong aria-live="polite">{loading ? '正在读取…' : rows.length + ' 部影片'}</strong>
              <label>
                <span className="sr-only">筛选资源状态</span>
                <select
                  aria-label="筛选资源状态"
                  value={filter}
                  onChange={(event) => {
                    setFilter(event.target.value);
                    setPage(0);
                  }}
                >
                  <option value="all">全部影片</option>
                  <option value="saved">已有资源</option>
                  <option value="pending">待确认资源</option>
                </select>
              </label>
            </div>
            {reviewError && filter === 'pending' && (
              <p className="field-error" role="alert">
                {reviewError}{' '}
                <button
                  type="button"
                  className="text-button"
                  onClick={() => setRevision((n) => n + 1)}
                >
                  重试读取候选
                </button>
              </p>
            )}
            {readError && (
              <p className="field-error" role="alert">
                {readError}{' '}
                <button
                  type="button"
                  className="text-button"
                  onClick={() => setRevision((n) => n + 1)}
                >
                  重试
                </button>
              </p>
            )}
            {!loading && !readError && rows.length === 0 && (
              <p className="field-help">
                {query || filter !== 'all'
                  ? '没有符合当前筛选的影片。'
                  : '本地片库为空，可更新豆瓣片库或导入片单。'}
              </p>
            )}
            <ul className="movie-list" aria-label="影片列表">
              {rows.slice(currentPage * pageSize, (currentPage + 1) * pageSize).map((item) => (
                <li key={item.id}>
                  <button
                    type="button"
                    aria-pressed={item.id === selected}
                    disabled={loading}
                    onClick={() => setSelected(item.id)}
                  >
                    <span>
                      <strong>{item.title}</strong>
                      <small>
                        {item.year} 年
                        {item.resource_count > 0 ? ' · ' + item.resource_count + ' 个资源' : ''}
                      </small>
                    </span>
                    <span
                      className="movie-rating"
                      aria-label={'豆瓣 ' + (item.rating_tenths / 10).toFixed(1) + ' 分'}
                    >
                      {(item.rating_tenths / 10).toFixed(1)}
                    </span>
                  </button>
                </li>
              ))}
            </ul>
            {lastPage > 0 && (
              <nav className="movie-pager" aria-label="片库分页">
                <button
                  type="button"
                  className="text-button"
                  disabled={currentPage === 0}
                  onClick={() => setPage(currentPage - 1)}
                >
                  上一页
                </button>
                <span>
                  {currentPage + 1} / {lastPage + 1}
                </span>
                <button
                  type="button"
                  className="text-button"
                  disabled={currentPage === lastPage}
                  onClick={() => setPage(currentPage + 1)}
                >
                  下一页
                </button>
              </nav>
            )}
          </section>
          {current ? (
            <MovieDetail
              key={current.id}
              movie={current}
              cacheRevision={revision}
              invoke={invoke}
              start={start}
              canStart={canStart}
              onClose={onClose}
              onSaved={() => setRevision((v) => v + 1)}
              onRatingRefreshed={(id, eligible, message) => {
                if (!alive.current) return;
                if (!eligible) {
                  setMovies((items) => items.filter((item) => item.id !== id));
                  setSelected((currentId) => (currentId === id ? '' : currentId));
                }
                setFeedback(message);
                setRevision((v) => v + 1);
              }}
            />
          ) : (
            <div className="movie-prompt">
              <strong>选择一部电影</strong>
              <p>查看评分来源、已收录版本，或按片名联网找资源。</p>
            </div>
          )}
        </div>
      </div>
    </Dialog>
  );
}
