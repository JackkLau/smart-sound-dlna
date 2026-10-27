export async function request(path, options = {}, timeout = 15000, fetcher = globalThis.fetch) {
  const controller = options.signal ? null : new AbortController();
  const timer = controller ? setTimeout(() => controller.abort(), timeout) : null;
  try {
    const response = await fetcher(path, {
      cache: 'no-store',
      ...options,
      signal: options.signal || controller.signal,
    });
    let data;
    try {
      data = await response.json();
    } catch {
      throw new Error('后台返回了无法读取的数据，请重新连接。');
    }
    if (!response.ok) {
      const error = new Error(data.error || `请求未完成（${response.status}），请重试。`);
      error.status = response.status;
      throw error;
    }
    return data;
  } catch (error) {
    if (error.name === 'AbortError' && controller)
      throw new Error('请求等待超时。操作可能已完成，请检查当前状态后再试。');
    if (error instanceof TypeError)
      throw new Error('连接中断。请确认后台程序和局域网连接，再重试。');
    throw error;
  } finally {
    if (timer) clearTimeout(timer);
  }
}

export function uploadFile(
  file,
  token,
  onProgress,
  onActive,
  makeXHR = () => new XMLHttpRequest(),
) {
  return new Promise((resolve, reject) => {
    const xhr = makeXHR();
    onActive(xhr);
    xhr.open('POST', '/api/library/upload');
    xhr.timeout = 600000;
    xhr.setRequestHeader('Content-Type', 'application/octet-stream');
    xhr.setRequestHeader('X-Control-Token', token);
    xhr.setRequestHeader('X-Filename', encodeURIComponent(file.name));
    xhr.upload.addEventListener('progress', (event) => {
      if (event.lengthComputable) onProgress(Math.round((event.loaded / event.total) * 100));
    });
    xhr.onload = () => {
      let result;
      try {
        result = JSON.parse(xhr.responseText);
      } catch {
        reject(new Error('上传结果无法读取，请检查音乐库后重试'));
        return;
      }
      if (xhr.status >= 200 && xhr.status < 300) resolve(result);
      else {
        const error = new Error(result.error || '上传未完成，请重试');
        error.status = xhr.status;
        reject(error);
      }
    };
    xhr.onerror = () => reject(new Error('连接中断，请检查后台程序后重试'));
    xhr.ontimeout = () => reject(new Error('上传超时，请检查音乐库后重试'));
    xhr.onabort = () => reject(new DOMException('已取消', 'AbortError'));
    xhr.send(file);
  });
}
