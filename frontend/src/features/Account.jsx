import React from 'react';
import { ErrorMessage } from './Form.jsx';

export default function Account({ provider, controller }) {
  const { features, pending, disabled, errors, run, invoke } = controller;
  const connected =
    provider === 'xiaomi' ? features.voice?.connected : features.downloads?.authenticated;
  const login = features[`${provider}_login`] || { phase: 'idle' };
  const waiting = ['waiting', 'checking', 'failed'].includes(login.phase);
  const checking = login.phase === 'checking';
  const accountBusy = pending === `${provider}-login`;
  const status = checking
    ? login.cached
      ? '正在恢复已保存的登录，请稍候…'
      : '正在确认登录状态，请稍候…'
    : login.phase === 'waiting'
      ? '请在官方窗口完成登录，成功后窗口会自动关闭。'
      : connected
        ? login.cached
          ? '已连接 · 重启后自动恢复登录'
          : '已连接 · 登录状态尚未保存'
        : login.cached
          ? '登录状态已保存 · 暂未连接，可重试或重新登录'
          : login.phase === 'cancelled'
            ? '登录已取消，可重新打开登录窗口。'
            : provider === 'xiaomi'
              ? '未连接 · 使用绑定音响的小米账号'
              : '未登录 · 公开视频可直接解析';
  const action = (command) => run(`${provider}-login`, () => invoke(command, { provider }));
  return (
    <>
      <div className="login-actions" data-provider={provider}>
        <button
          type="button"
          className="button outline"
          disabled={disabled || checking}
          aria-busy={accountBusy}
          onClick={() => action('open_provider_login')}
        >
          {connected || login.cached
            ? '重新登录'
            : provider === 'xiaomi'
              ? '登录小米账号'
              : '登录 Bilibili'}
        </button>
        {(waiting || (login.cached && !connected)) && (
          <button
            type="button"
            className="button outline"
            disabled={disabled || checking}
            aria-busy={accountBusy}
            onClick={() => action('finish_provider_login')}
          >
            {checking ? '正在确认…' : '重试连接'}
          </button>
        )}
        {(connected || waiting || login.cached) && (
          <button
            type="button"
            className="text-button"
            disabled={disabled || checking}
            onClick={() => action('disconnect_provider')}
          >
            退出登录
          </button>
        )}
      </div>
      <p id={`${provider}-account-status`} className="field-help" role="status">
        {status}
      </p>
      <ErrorMessage id={`${provider}-login-error`}>
        {errors[`${provider}-login`] || login.error}
      </ErrorMessage>
    </>
  );
}
