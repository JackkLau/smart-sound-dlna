import React, { useEffect, useMemo, useRef, useState } from 'react';
import qrcode from 'qrcode-generator';
import { Dialog, Select } from '../components.jsx';
import { messageOf } from '../utils.js';
import { Form, ErrorMessage } from './Form.jsx';

export default function MobilePanel({ open, onClose, controller }) {
  const { pending, disabled, errors, run, invoke } = controller;
  const [receiver, setReceiver] = useState({ enabled: false, entries: [] });
  const [address, setAddress] = useState('');
  const [pollError, setPollError] = useState('');
  const revision = useRef(0);
  const polling = useRef(false);
  useEffect(() => {
    if (!open || disabled) return;
    let active = true;
    const poll = async () => {
      if (polling.current) return;
      polling.current = true;
      const current = revision.current;
      try {
        const next = await invoke('get_mobile_receiver');
        if (active && current === revision.current) {
          setReceiver(next);
          setPollError('');
        }
      } catch (error) {
        if (active) setPollError(messageOf(error));
      } finally {
        polling.current = false;
      }
    };
    poll();
    const timer = setInterval(poll, 2000);
    return () => {
      active = false;
      clearInterval(timer);
      revision.current += 1;
    };
  }, [open, disabled, invoke]);
  const entry = receiver.entries.find((item) => item.address === address) || receiver.entries[0];
  const url = receiver.enabled && (receiver.paired || receiver.pairing_available) ? entry?.url : '';
  const qrSource = useMemo(() => {
    if (!url) return '';
    const qr = qrcode(0, 'M');
    qr.addData(url);
    qr.make();
    return qr.createDataURL(6, 24);
  }, [url]);
  const change = (command, args) =>
    run('mobile-form', async () => {
      revision.current += 1;
      setReceiver(await invoke(command, args));
      setPollError('');
    });
  return (
    <Dialog
      id="mobile-dialog"
      title="手机点歌与导入"
      eyebrow="手机点歌 · 电脑推送"
      description="mobile-description"
      open={open}
      onClose={onClose}
      busy={Boolean(pending)}
      className="feature-dialog"
    >
      <Form
        noValidate
        id="mobile-form"
        className="dialog-body"
        onSubmit={() => change('set_mobile_receiver', { enabled: !receiver.enabled })}
      >
        <p id="mobile-description" className="dialog-description">
          手机与电脑连接同一 Wi-Fi。扫码后可选择电脑曲库中的歌曲、设置播放模式，也能粘贴 Bilibili
          分享链接下载音轨，无需安装安卓应用。
        </p>
        {receiver.enabled && (
          <div id="mobile-details">
            {receiver.entries.length > 1 && (
              <div>
                <label className="field-label" htmlFor="mobile-network">
                  电脑网络（选择与手机同一网段的 IP）
                </label>
                <Select
                  id="mobile-network"
                  className="text-input"
                  value={entry?.address || ''}
                  disabled={disabled}
                  onChange={(event) => setAddress(event.target.value)}
                >
                  {receiver.entries.map((item) => (
                    <option key={item.address} value={item.address}>
                      {item.address}
                    </option>
                  ))}
                </Select>
              </div>
            )}
            <p id="mobile-status" className="field-help" role="status">
              {receiver.paired
                ? '已记住手机。用配对时的浏览器打开收藏或桌面快捷方式，也可扫描下方二维码。换手机或浏览器时，请先忘记手机并重新配对。'
                : receiver.pairing_available
                  ? '用手机相机扫码，在常用浏览器打开，自动完成配对，无需输入连接码。二维码 10 分钟内有效。'
                  : '配对二维码已失效，请点击“忘记手机并重新配对”生成新的二维码。'}
            </p>
            {qrSource && (
              <div className="mobile-qr-frame">
                <img
                  id="mobile-qr"
                  src={qrSource}
                  width={264}
                  height={264}
                  alt="用手机扫码打开手机点歌页面"
                  aria-describedby="mobile-status"
                />
              </div>
            )}
            <p className="field-help">
              首次打开后，收藏此页或通过浏览器菜单“添加到主屏幕”。下次直接点开，不必再次扫码。请使用同一个浏览器，避免在聊天应用的内置浏览器中配对。
            </p>
          </div>
        )}
        <p className="field-help">
          只在可信的家庭网络开启。配对手机可浏览曲名、控制电脑已选择的音响，并发送视频下载；不会获得账号登录信息或电脑文件路径。“忘记手机并重新配对”会让原手机失去访问权限。
        </p>
        <p className="field-help">
          手机打不开页面时，请检查两端
          Wi-Fi、路由器访客隔离，以及电脑防火墙是否允许此应用接收局域网连接。
        </p>
        <ErrorMessage id="mobile-form-error">{errors['mobile-form'] || pollError}</ErrorMessage>
        <div className="dialog-actions">
          <button
            type="button"
            className="button outline"
            disabled={disabled}
            onClick={() => change('forget_mobile_pairing', {})}
          >
            忘记手机并重新配对
          </button>
          <button
            type="submit"
            className="button primary"
            disabled={disabled}
            aria-busy={pending === 'mobile-form'}
          >
            {receiver.enabled ? '暂停手机接收' : '开启手机接收'}
          </button>
        </div>
      </Form>
    </Dialog>
  );
}
