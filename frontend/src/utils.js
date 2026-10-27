export function messageOf(error) {
  if (typeof error === 'string' && error.trim()) return error;
  if (error && typeof error.message === 'string') return error.message;
  return '操作未完成，请重试；如果问题持续，请重新启动客户端。';
}

export function formatSize(bytes) {
  const value = Math.max(0, Number(bytes) || 0);
  return value < 1048576 ? `${(value / 1024).toFixed(0)} KB` : `${(value / 1048576).toFixed(1)} MB`;
}

export function formatTime(seconds) {
  const value = Math.max(0, Math.floor(Number(seconds) || 0));
  const hours = Math.floor(value / 3600);
  return hours
    ? `${hours}:${String(Math.floor(value / 60) % 60).padStart(2, '0')}:${String(value % 60).padStart(2, '0')}`
    : `${Math.floor(value / 60)}:${String(value % 60).padStart(2, '0')}`;
}
