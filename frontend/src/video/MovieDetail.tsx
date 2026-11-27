import { useEffect, useRef, useState } from 'react';
import type { Invoke } from './types';
import { videoMessage } from './useVideo';

export type Movie = {
  id: string;
  title: string;
  original: string;
  year: number;
  rating_tenths: number;
  observed: string;
  rating_evidence?: string | null;
  resource_count: number;
};
type Version = { uri: string; source: string; sources?: string[]; license_url?: string | null };
type Candidate = Version & { title: string; reason: string };
export type Start = (id: string, uri: string) => Promise<boolean>;

function versionName(uri: string) {
  const params = new URL(uri).searchParams;
  return params.get('dn') || params.get('xt')?.replace(/^urn:btih:/i, '') || '磁力资源';
}

export default function MovieDetail({
  movie,
  invoke,
  start,
  canStart,
  onSaved,
  onRatingRefreshed,
  cacheRevision,
  onClose,
}: {
  movie: Movie;
  invoke: Invoke;
  start: Start;
  canStart: boolean;
  onSaved: () => void;
  onRatingRefreshed: (id: string, eligible: boolean, message: string) => void;
  cacheRevision: number;
  onClose: () => void;
}) {
  const [versions, setVersions] = useState<Version[] | null>(null);
  const [candidates, setCandidates] = useState<Candidate[]>([]);
  const [manual, setManual] = useState<Candidate[]>([]);
  const cacheEpoch = useRef(0);
  const [draft, setDraft] = useState('');
  const [removing, setRemoving] = useState('');
  const [error, setError] = useState('');
  const [status, setStatus] = useState('');
  const [pending, setPending] = useState('');
  const lock = useRef(false);
  const mounted = useRef(false);
  const heading = useRef<HTMLHeadingElement>(null);
  const [revision, setRevision] = useState(0);
  useEffect(() => {
    mounted.current = true;
    heading.current?.focus({ preventScroll: true });
    return () => {
      mounted.current = false;
    };
  }, []);
  useEffect(() => {
    let active = true;
    invoke('movie_library_links', { id: movie.id })
      .then((value) => {
        if (active) setVersions(value as Version[]);
      })
      .catch((failure) => {
        if (active) setError(videoMessage(failure));
      });
    return () => {
      active = false;
    };
  }, [invoke, movie.id, revision]);

  useEffect(() => {
    const epoch = ++cacheEpoch.current;
    invoke('movie_library_candidates', { id: movie.id })
      .then((value) => {
        if (mounted.current && epoch === cacheEpoch.current) setCandidates(value as Candidate[]);
      })
      .catch((failure) => {
        if (mounted.current && epoch === cacheEpoch.current) setError(videoMessage(failure));
      });
    return () => {
      cacheEpoch.current++;
    };
  }, [invoke, movie.id, cacheRevision]);
  const pendingCandidates = [...candidates, ...manual];

  async function act(action: string, perform: () => Promise<void>) {
    if (lock.current) return;
    lock.current = true;
    setPending(action);
    setError('');
    setStatus('');
    try {
      await perform();
    } catch (failure) {
      if (mounted.current) setError(videoMessage(failure));
    } finally {
      lock.current = false;
      if (mounted.current) setPending('');
    }
  }
  const openSource = (url: string) =>
    act('open', async () => {
      await invoke('movie_library_open_source', { url });
    });
  const refreshRating = () =>
    act('rating', async () => {
      const result = (await invoke('movie_library_refresh', { id: movie.id })) as {
        eligible: boolean;
        message: string;
      };
      onRatingRefreshed(movie.id, result.eligible, result.message);
    });
  const search = () =>
    act('search', async () => {
      cacheEpoch.current++;
      const found = (await invoke('movie_library_search', { id: movie.id })) as Candidate[];
      if (!mounted.current) return;
      setCandidates(found);
      setStatus(
        found.length ? '请核对候选版本，再决定是否收录。' : '此次搜索没有找到可收录的版本。',
      );
    });
  const parse = (text?: string) =>
    act('paste', async () => {
      const found = (
        text === undefined
          ? await invoke('read_magnet_links')
          : await invoke('read_magnet_links', { text })
      ) as { uri: string; title: string }[];
      if (!mounted.current) return;
      setManual(
        found.map((item) => ({
          ...item,
          source: 'manual',
          reason: '手动提供，请核对影片与版本。',
        })),
      );
      setStatus(found.length ? '已识别磁力链接，请核对后收录。' : '未识别到有效的磁力链接。');
    });
  const save = (item: Candidate) =>
    act('save', async () => {
      await invoke('movie_library_save', { id: movie.id, uri: item.uri, source: item.source });
      if (!mounted.current) return;
      setManual((previous) =>
        previous.filter((row) => row.uri !== item.uri || row.source !== item.source),
      );
      setCandidates((previous) =>
        previous.filter((row) => row.uri !== item.uri || row.source !== item.source),
      );
      setRevision((value) => value + 1);
      onSaved();
      setStatus('版本已收录。');
    });
  const dismiss = (item: Candidate) =>
    act('dismiss', async () => {
      if (item.source !== 'manual')
        await invoke('movie_library_dismiss', { id: movie.id, uri: item.uri });
      if (!mounted.current) return;
      setManual((previous) => previous.filter((row) => row.uri !== item.uri));
      setCandidates((previous) => previous.filter((row) => row.uri !== item.uri));
      onSaved();
      setStatus('已忽略此候选。');
    });
  const detach = (uri: string) =>
    act('remove', async () => {
      await invoke('movie_library_remove', { id: movie.id, uri });
      if (!mounted.current) return;
      setRemoving('');
      setRevision((n) => n + 1);
      onSaved();
      setStatus('已移除影片与此版本的关联，本地文件保留。');
    });
  const download = (uri: string) =>
    act('start', async () => {
      const started = await start(movie.id, uri);
      if (!mounted.current) return;
      if (started) onClose();
      else setStatus('已取消目录选择。');
    });
  return (
    <section className="movie-detail" aria-label="影片详情">
      <div className="movie-detail-heading">
        <div>
          <h3 ref={heading} tabIndex={-1} dir="auto">
            {movie.title}
          </h3>
          {movie.original && movie.original !== movie.title && (
            <p lang="und" dir="auto">
              {movie.original}
            </p>
          )}
        </div>
        <strong
          className="movie-score"
          aria-label={'豆瓣评分 ' + (movie.rating_tenths / 10).toFixed(1)}
        >
          {(movie.rating_tenths / 10).toFixed(1)}
        </strong>
      </div>
      <p className="field-help">
        {movie.year} 年 · 评分采集于 {movie.observed}
      </p>
      <button
        type="button"
        className="text-button"
        disabled={!!pending}
        onClick={() => void refreshRating()}
      >
        {pending === 'rating' ? '正在刷新评分…' : '刷新此片评分'}
      </button>
      <button
        type="button"
        className="text-button"
        disabled={!!pending}
        onClick={() => void openSource('https://movie.douban.com/subject/' + movie.id + '/')}
      >
        查看豆瓣条目
      </button>
      {movie.rating_evidence && (
        <button
          type="button"
          className="text-button"
          disabled={!!pending}
          onClick={() => void openSource(movie.rating_evidence!)}
        >
          查看评分快照
        </button>
      )}
      <div className="movie-section-heading">
        <h4>已收录版本</h4>
        <button
          type="button"
          className="button outline"
          disabled={!!pending}
          onClick={() => void search()}
        >
          {pending === 'search' ? '正在搜索…' : '联网找资源'}
        </button>
      </div>
      {versions === null ? (
        !error && <p className="field-help">正在读取版本…</p>
      ) : versions.length ? (
        <ul className="movie-versions" aria-label="已收录版本">
          {versions.map((item) => (
            <li key={item.uri + ' ' + item.source}>
              <strong dir="auto">{versionName(item.uri)}</strong>
              <div className="movie-actions">
                {(item.sources || [item.source]).map((source, index) =>
                  source === 'manual' ? (
                    <span key={source} className="field-help">
                      手动收录
                    </span>
                  ) : (
                    <button
                      key={source}
                      type="button"
                      className="text-button"
                      title={source}
                      disabled={!!pending}
                      onClick={() => void openSource(source)}
                    >
                      {(item.sources?.length ?? 0) > 1 ? '查看来源 ' + (index + 1) : '查看来源'}
                    </button>
                  ),
                )}
                <button
                  type="button"
                  className="button outline"
                  disabled={!canStart || !!pending}
                  onClick={() => void download(item.uri)}
                >
                  选择目录并下载
                </button>
                {item.license_url && (
                  <button
                    type="button"
                    className="text-button"
                    disabled={!!pending}
                    onClick={() => void openSource(item.license_url!)}
                  >
                    查看许可
                  </button>
                )}
                {removing !== item.uri && (
                  <button
                    type="button"
                    className="text-button"
                    disabled={!!pending}
                    onClick={() => setRemoving(item.uri)}
                  >
                    移除关联
                  </button>
                )}
              </div>
              <details className="movie-uri">
                <summary>查看磁力链接</summary>
                <input
                  type="text"
                  className="text-input"
                  aria-label="已收录的磁力链接"
                  readOnly
                  value={item.uri}
                  onFocus={(event) => event.currentTarget.select()}
                />
              </details>
              {removing === item.uri && (
                <div className="movie-remove" role="group" aria-label="确认移除版本">
                  <p>从《{movie.title}》移除此版本的关联？本地文件和已开始的下载会保留。</p>
                  <div className="movie-actions">
                    <button
                      type="button"
                      className="button outline"
                      disabled={!!pending}
                      onClick={() => setRemoving('')}
                    >
                      取消
                    </button>
                    <button
                      type="button"
                      className="button danger"
                      disabled={!!pending}
                      onClick={() => void detach(item.uri)}
                    >
                      确认移除关联
                    </button>
                  </div>
                </div>
              )}
            </li>
          ))}
        </ul>
      ) : (
        <p className="field-help">暂无已收录版本。</p>
      )}
      {!!versions?.length && <p className="field-help">资源可用性需在连接下载节点后确认。</p>}
      {!canStart && !!versions?.length && (
        <p className="field-help">下载器就绪且当前任务结束后可开始下载。</p>
      )}
      {versions === null && error && (
        <button
          type="button"
          className="text-button"
          onClick={() => {
            setError('');
            setRevision((n) => n + 1);
          }}
        >
          重试读取版本
        </button>
      )}
      <details className="movie-manual">
        <summary>从剪贴板或文本添加</summary>
        <label className="field-label" htmlFor="movie-share">
          磁力链接或分享文本
        </label>
        <textarea
          id="movie-share"
          className="text-input"
          rows={2}
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          spellCheck={false}
          maxLength={256 * 1024}
        />
        <div className="movie-actions">
          <button
            type="button"
            className="button outline"
            disabled={!draft.trim() || !!pending}
            onClick={() => void parse(draft)}
          >
            解析文本
          </button>
          <button
            type="button"
            className="text-button"
            disabled={!!pending}
            onClick={() => void parse()}
          >
            读取剪贴板
          </button>
        </div>
      </details>
      {pendingCandidates.length > 0 && (
        <section className="movie-candidates" aria-label="候选版本">
          <h4>待确认版本</h4>
          <ul className="movie-versions">
            {pendingCandidates.map((item) => (
              <li key={item.uri + ' ' + item.source}>
                <strong dir="auto">{item.title || versionName(item.uri)}</strong>
                <p className="field-help">{item.reason}</p>
                <div className="movie-actions">
                  {item.source !== 'manual' && (
                    <button
                      type="button"
                      className="text-button"
                      disabled={!!pending}
                      onClick={() => void openSource(item.source)}
                    >
                      核对发布页
                    </button>
                  )}
                  {item.license_url && (
                    <button
                      type="button"
                      className="text-button"
                      disabled={!!pending}
                      onClick={() => void openSource(item.license_url!)}
                    >
                      核对许可
                    </button>
                  )}
                  <button
                    type="button"
                    className="button outline"
                    disabled={!!pending}
                    onClick={() => void save(item)}
                  >
                    收录此版本
                  </button>
                  <button
                    type="button"
                    className="text-button"
                    disabled={!!pending}
                    onClick={() => void dismiss(item)}
                  >
                    忽略此候选
                  </button>
                </div>
              </li>
            ))}
          </ul>
        </section>
      )}
      {status && (
        <p className="field-help" role="status">
          {status}
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
