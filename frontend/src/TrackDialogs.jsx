import { useEffect, useRef, useState } from 'react';
import { Dialog } from './components.jsx';
import { messageOf } from './utils.js';

export function TrackDialogs({ active, onClose, mutate, invoke, announce }) {
  const drafts = useRef(new Map());
  if (!active) return null;
  return (
    <TrackDialog
      key={`${active.mode}:${active.track.id}`}
      active={active}
      drafts={drafts.current}
      onClose={onClose}
      mutate={mutate}
      invoke={invoke}
      announce={announce}
    />
  );
}
function TrackDialog({ active, drafts, onClose, mutate, invoke, announce }) {
  const { track, mode } = active;
  const draft = drafts.get(track.id);
  const [name, setName] = useState(
    draft?.base === track.title ? draft.value : track.title || track.filename,
  );
  const [error, setError] = useState('');
  const [pending, setPending] = useState('');
  const [needsFocus, setNeedsFocus] = useState(false);
  const [recognition, setRecognition] = useState('从音频标签、文件名或视频标题提取，不上传音频。');
  const field = useRef(null);
  const cancel = useRef(null);
  const lock = useRef(false);
  const composing = useRef(false);
  const close = () => {
    if (lock.current) return;
    if (mode === 'edit') drafts.set(track.id, { value: name, base: track.title });
    onClose(false);
  };
  useEffect(() => {
    if (!pending && needsFocus) {
      (mode === 'edit' ? field.current : cancel.current)?.focus();
      setNeedsFocus(false);
    }
  }, [pending, needsFocus, mode]);
  const fail = (failure) => {
    setError(messageOf(failure));
    setNeedsFocus(true);
  };
  const recognize = async () => {
    if (lock.current) return;
    lock.current = true;
    setPending('recognize');
    setError('');
    setRecognition('正在读取曲名信息…');
    try {
      const result = await invoke('recognize_track_name', { trackId: track.id });
      if (result.title) {
        setName(result.title);
        setRecognition(
          `已从${{ audio_tag: '音频标签', video_title: '视频标题', filename: '文件名' }[result.source] || '本地信息'}提取。请核对曲名，保存后生效。`,
        );
      } else setRecognition('未找到可靠曲名，请手动输入。');
    } catch (failure) {
      setRecognition('识别未完成，可以重试或直接修改曲名。');
      fail(failure);
    } finally {
      lock.current = false;
      setPending('');
      setNeedsFocus(true);
    }
  };
  const submit = async (event) => {
    event.preventDefault();
    if (lock.current || composing.current || event.nativeEvent?.isComposing) return;
    const title = name.trim();
    if (
      mode === 'edit' &&
      (!/[\p{L}\p{N}]/u.test(title) ||
        [...title].length > 120 ||
        /[\u0000-\u001f\u007f-\u009f]/u.test(title))
    ) {
      fail('请输入 1–120 个字符的曲名，包含文字或数字，不含换行。');
      return;
    }
    lock.current = true;
    setPending('save');
    setError('');
    try {
      const result = await mutate(
        mode === 'edit' ? 'rename_track' : 'remove_track',
        mode === 'edit'
          ? { trackId: track.id, title, expectedTitle: track.title }
          : { trackId: track.id },
        '',
        { scopedError: true },
      );
      if (!result) throw new Error('客户端暂未就绪，请稍后重试。');
      drafts.delete(track.id);
      announce(
        mode === 'edit'
          ? `已保存曲名“${title}”，可说“播放本地 ${title}”`
          : `已将“${track.title || track.filename}”移出曲库，音频文件已保留`,
      );
      onClose(true);
    } catch (failure) {
      fail(failure);
    } finally {
      lock.current = false;
      setPending('');
    }
  };
  const id = mode === 'edit' ? 'track-name-dialog' : 'remove-track-dialog';
  return (
    <Dialog
      id={id}
      title={mode === 'edit' ? '识别或修改曲名' : '移出这首音乐？'}
      eyebrow={mode === 'edit' ? '用你熟悉的歌名点播' : '管理你的曲库'}
      open
      busy={Boolean(pending)}
      onClose={close}
      description={mode === 'edit' ? 'track-name-description' : 'remove-track-description'}
    >
      <form
        id={mode === 'edit' ? 'track-name-form' : 'remove-track-form'}
        className="dialog-body"
        noValidate
        onSubmit={submit}
        onCompositionStart={() => {
          composing.current = true;
        }}
        onCompositionEnd={() => {
          composing.current = false;
        }}
      >
        {mode === 'edit' ? (
          <>
            <p id="track-name-description" className="dialog-description">
              识别结果不准确时，可直接修改。保存后，曲库和语音点播都会使用新名称。
            </p>
            <label className="field-label" htmlFor="track-name">
              曲名
            </label>
            <input
              ref={field}
              autoFocus
              data-autofocus
              className="text-input"
              id="track-name"
              name="title"
              autoComplete="off"
              maxLength="240"
              required
              disabled={Boolean(pending)}
              value={name}
              onChange={(event) => {
                setName(event.target.value);
                setError('');
              }}
              aria-invalid={Boolean(error)}
              aria-describedby="track-name-help track-name-form-error"
            />
            <p className="field-help" id="track-name-help">
              最多 120 个字符。同一首歌的不同版本，可在名称中加上歌手或版本。
            </p>
            <div className="name-recognition">
              <button
                type="button"
                className="button outline"
                id="recognize-track-name"
                disabled={Boolean(pending)}
                aria-busy={pending === 'recognize'}
                onClick={recognize}
              >
                {pending === 'recognize' ? '识别中…' : '识别曲名'}
              </button>
              <p className="field-help" id="track-name-recognition" role="status">
                {recognition}
              </p>
            </div>
            <p className="field-help name-voice-preview" id="track-name-voice" aria-live="polite">
              {name.trim()
                ? `保存后可说：“小爱同学，播放本地 ${name.trim()}”。请先在“小爱语音”中开始接收指令。`
                : '填写曲名后，即可按这个名称语音点播。'}
            </p>
            <p className="field-help" id="track-name-file">
              原文件：{track.filename}
            </p>
          </>
        ) : (
          <>
            <p id="remove-track-name" className="remove-track-name">
              {track.title || track.filename}
            </p>
            <p id="remove-track-description" className="dialog-description">
              音频文件会保留，可重新导入；手动修改的曲名需要重新设置。如果这首音乐正在播放，会先停止播放。
            </p>
          </>
        )}
        <p
          id={mode === 'edit' ? 'track-name-form-error' : 'remove-track-form-error'}
          className="field-error"
          role="alert"
        >
          {error}
        </p>
        <div className="dialog-actions">
          <button
            ref={cancel}
            autoFocus={mode === 'remove'}
            data-autofocus={mode === 'remove' ? 'true' : undefined}
            type="button"
            className="button outline"
            id={mode === 'edit' ? 'track-name-cancel' : 'remove-track-cancel'}
            disabled={Boolean(pending)}
            onClick={close}
          >
            取消
          </button>
          <button
            type="submit"
            className={`button ${mode === 'edit' ? 'primary' : 'danger'}`}
            disabled={Boolean(pending)}
            aria-busy={pending === 'save'}
          >
            {pending === 'save'
              ? mode === 'edit'
                ? '保存中…'
                : '移除中…'
              : mode === 'edit'
                ? '保存曲名'
                : '移出曲库'}
          </button>
        </div>
      </form>
    </Dialog>
  );
}
