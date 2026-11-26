import { useCallback, useEffect, useRef, useState } from 'react';

// Read on entry and foreground return. An edit, paste, blur, or close invalidates
// any older read so that a late clipboard reply cannot change the user's choice.
export default function useMagnetClipboard({ open, invoke, field, clearError, disabled }) {
  const [uri, setUri] = useState('');
  const [candidates, setCandidates] = useState([]);
  const [notice, setNotice] = useState('');
  const revision = useRef(0);
  const blocked = useRef(disabled);
  blocked.current = disabled;
  const autoValue = useRef(null);
  const lastFilled = useRef(null);
  const draft = useRef('');
  const clearFormError = useRef(clearError);
  clearFormError.current = clearError;

  const edit = useCallback((value) => {
    revision.current++;
    autoValue.current = null;
    draft.current = value;
    setUri(value);
    setNotice('');
    clearFormError.current('magnet-form');
  }, []);

  const read = useCallback(
    async (text) => {
      if (blocked.current) return;
      const ticket = ++revision.current;
      const pasted = text !== undefined;
      try {
        const found = pasted
          ? await invoke('read_magnet_links', { text })
          : await invoke('read_magnet_links');
        if (revision.current !== ticket || blocked.current) return;
        setCandidates(found);
        if (!found.length) {
          setNotice('未识别到支持的磁力链接，可手动粘贴。');
          return;
        }
        const first = found[0].uri;
        const canFill =
          pasted ||
          draft.current === autoValue.current ||
          (!draft.current && lastFilled.current !== first);
        if (found.length === 1 && canFill) {
          edit(first);
          autoValue.current = first;
          lastFilled.current = first;
          setNotice('已识别并填入 1 条磁力链接。');
        } else {
          if (found.length > 1 && autoValue.current && draft.current === autoValue.current)
            edit('');
          setNotice(`识别到 ${found.length} 个资源，可选择要下载的链接。`);
        }
      } catch (error) {
        if (revision.current !== ticket || blocked.current) return;
        setCandidates([]);
        setNotice(typeof error === 'string' ? error : '无法读取剪贴板，可手动粘贴或重新读取。');
      }
    },
    [edit, invoke],
  );

  useEffect(() => {
    if (!open || !invoke) return;
    const foreground = () => {
      if (document.visibilityState !== 'hidden') void read();
    };
    const invalidate = () => {
      revision.current++;
    };
    foreground();
    window.addEventListener('focus', foreground);
    window.addEventListener('blur', invalidate);
    return () => {
      invalidate();
      window.removeEventListener('focus', foreground);
      window.removeEventListener('blur', invalidate);
    };
  }, [open, invoke, read]);

  const onPaste = (event) => {
    const text = event.clipboardData?.getData('text/plain');
    if (!text || !/magnet:\?/i.test(text)) return;
    event.preventDefault();
    if (text.length > 256 * 1024) {
      revision.current++;
      setCandidates([]);
      setNotice('文本过长，请只复制包含磁力链接的段落。');
      return;
    }
    void read(text);
  };
  const choose = (candidate) => {
    edit(candidate.uri);
    lastFilled.current = candidate.uri;
    field.current?.focus();
  };
  const submitted = (value) => {
    edit('');
    lastFilled.current = value;
    setCandidates((previous) => previous.filter((item) => item.uri !== value));
  };
  return { uri, edit, read, onPaste, choose, submitted, candidates, notice };
}
