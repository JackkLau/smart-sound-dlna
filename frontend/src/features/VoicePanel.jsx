import React, { useRef, useState } from 'react';
import { Dialog, Select } from '../components.jsx';
import { Form, ErrorMessage } from './Form.jsx';
import Account from './Account.jsx';
import { emptyFeatures } from './useFeatureController.js';

export default function VoicePanel({ open, onClose, controller }) {
  const { features, disabled, pending, errors, run, invoke, refresh, announce } = controller;
  const voice = features.voice || emptyFeatures.voice;
  const [source, setSource] = useState('');
  const [text, setText] = useState('');
  const [result, setResult] = useState('');
  const [cookie, setCookie] = useState('');
  const [showCookie, setShowCookie] = useState(false);
  const sourceField = useRef(null);
  const textField = useRef(null);
  const cookieField = useRef(null);
  const preferredSource = source || voice.source_id;
  const sourceId = (voice.devices || []).some((device) => device.id === preferredSource)
    ? preferredSource
    : voice.devices?.[0]?.id || '';
  return (
    <Dialog
      id="voice-dialog"
      title="小爱语音点播"
      eyebrow="开口，播放你的收藏"
      description="voice-description"
      open={open}
      onClose={onClose}
      busy={Boolean(pending)}
      className="feature-dialog"
    >
      <div className="dialog-body">
        <p className="dialog-description" id="voice-description">
          连接绑定音响的小米账号，开启接收后读取新的本地点播指令。登录状态由系统安全保存，重启后自动恢复；电脑和音响需保持联网。
        </p>
        <Account provider="xiaomi" controller={controller} />
        <Form
          noValidate
          id="voice-bind-form"
          onSubmit={() =>
            run(
              'voice-bind-form',
              async () => {
                if (!features.selected_device_id) throw new Error('请先在主界面选择播放音响。');
                if (!sourceId) throw new Error('请选择接收语音的小爱音响。');
                await invoke('enable_voice', {
                  sourceId,
                  targetId: features.selected_device_id,
                });
                announce('已开始读取新的本地点播指令');
              },
              sourceField,
            )
          }
        >
          <label className="field-label" htmlFor="voice-source">
            听取哪台小爱的指令
          </label>
          <Select
            id="voice-source"
            ref={sourceField}
            className="text-input"
            value={sourceId}
            disabled={disabled || !voice.connected}
            onChange={(event) => setSource(event.target.value)}
            aria-invalid={Boolean(errors['voice-bind-form'])}
            aria-describedby="voice-target voice-bind-form-error"
          >
            {!voice.devices?.length && <option value="">登录后选择音响</option>}
            {(voice.devices || []).map((device) => (
              <option key={device.id} value={device.id}>
                {device.name} · {device.hardware}
              </option>
            ))}
          </Select>
          <p id="voice-target" className="field-help">
            播放到：{features.selected_device_name || '请先在主界面连接音响'}
            {voice.listening && voice.target_id !== features.selected_device_id
              ? '（音响已切换，需更新绑定）'
              : ''}
          </p>
          <ErrorMessage id="voice-bind-form-error">{errors['voice-bind-form']}</ErrorMessage>
          <div className="dialog-actions">
            <button
              type="button"
              className="button outline"
              disabled={disabled || !voice.listening}
              onClick={() => run('voice-bind-form', () => invoke('disable_voice'))}
            >
              停止接收
            </button>
            <button
              className="button primary"
              type="submit"
              disabled={disabled || !voice.connected || !features.selected_device_id}
              aria-busy={pending === 'voice-bind-form'}
            >
              {voice.listening ? '更新绑定' : '开始接收指令'}
            </button>
          </div>
        </Form>
        <div className="voice-status">
          <strong>
            {voice.listening
              ? voice.last_poll_ms
                ? '正在接收本地点播指令'
                : '正在首次读取语音记录…'
              : '尚未开始接收'}
          </strong>
          <p className="field-help" role="status">
            {voice.last_command
              ? `「${voice.last_command}」：${voice.last_result}`
              : voice.listening
                ? '只处理启用后出现的新指令，历史对话不会执行。'
                : '使用下方测试入口，可先检查点歌指令。'}
          </p>
          <p id="voice-runtime-error" className="field-error" role="status">
            {voice.error || errors.poll}
          </p>
        </div>
        <div className="command-guide">
          <strong>先说“小爱同学”，再说：</strong>
          <p>
            播放本地 + 歌名 · 本地下一首 · 本地上一首
            <br />
            本地暂停 · 本地继续 · 本地停止 · 本地音量三十
          </p>
          <p className="field-help">
            还支持：本地单曲循环 / 列表循环 / 顺序播放。歌名有多个匹配时会提示说得更完整。
          </p>
        </div>
        <Form
          noValidate
          id="voice-test-form"
          onSubmit={() =>
            run(
              'voice-test-form',
              async () => {
                setResult('');
                setResult(await invoke('voice_command', { text }));
                await refresh();
              },
              textField,
            )
          }
        >
          <label className="field-label" htmlFor="voice-text">
            测试指令
          </label>
          <div className="inline-field">
            <input
              id="voice-text"
              ref={textField}
              className="text-input"
              placeholder="播放本地 晴天"
              maxLength={300}
              disabled={disabled}
              value={text}
              onChange={(event) => setText(event.target.value)}
              aria-invalid={Boolean(errors['voice-test-form'])}
              aria-describedby="voice-test-help voice-test-form-error"
            />
            <button
              className="button outline"
              type="submit"
              disabled={disabled}
              aria-busy={pending === 'voice-test-form'}
            >
              执行指令
            </button>
          </div>
          <p id="voice-test-help" className="field-help">
            使用与小爱语音相同的指令处理，会实际控制当前音响。
          </p>
          <p id="voice-test-result" className="field-help" role="status">
            {result}
          </p>
          <ErrorMessage id="voice-test-form-error">{errors['voice-test-form']}</ErrorMessage>
        </Form>
        <details className="feature-details">
          <summary>登录遇到问题？使用已有会话</summary>
          <p className="field-help">
            高级接入：可粘贴自己小爱服务会话中的 userId 和
            serviceToken（micoapi）。验证成功后由系统安全保存，重启后恢复；退出登录会清除，不写入曲库或日志。
          </p>
          <Form
            noValidate
            id="voice-session-form"
            onSubmit={() =>
              run(
                'voice-session-form',
                async () => {
                  const submitted = cookie;
                  setCookie('');
                  await invoke('connect_voice_session', { cookie: submitted });
                  announce('已连接小米账号，请选择接收语音的音响');
                },
                cookieField,
              )
            }
          >
            <label className="field-label" htmlFor="voice-cookie">
              小爱服务 Cookie
            </label>
            <input
              id="voice-cookie"
              ref={cookieField}
              className="text-input"
              type={showCookie ? 'text' : 'password'}
              autoComplete="off"
              maxLength={16384}
              disabled={disabled}
              placeholder="userId=…; serviceToken=…"
              value={cookie}
              onChange={(event) => setCookie(event.target.value)}
              aria-describedby="voice-session-form-error"
            />
            <button
              type="button"
              className="text-button"
              aria-pressed={showCookie}
              disabled={disabled}
              onClick={() => setShowCookie((value) => !value)}
            >
              {showCookie ? '隐藏会话' : '显示会话'}
            </button>
            <ErrorMessage id="voice-session-form-error">
              {errors['voice-session-form']}
            </ErrorMessage>
            <div className="dialog-actions">
              <button
                className="button outline"
                type="submit"
                disabled={disabled}
                aria-busy={pending === 'voice-session-form'}
              >
                连接会话
              </button>
            </div>
          </Form>
        </details>
        <p className="field-help dialog-footnote">
          此功能依赖小米云端对话记录，可能有数秒延迟。账号、固件或接口变化可能影响接入；未读取到记录时会明确提示。重启后登录自动恢复，需重新选择音响并开启接收。
        </p>
      </div>
    </Dialog>
  );
}
