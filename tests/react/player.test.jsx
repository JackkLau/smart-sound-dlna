import React from 'react';
import { act, fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import Player from '../../frontend/src/Player.jsx';

const initial = {
  tracks: [{ id: 'song', title: '晴天', filename: '晴天.mp3' }],
  devices: [{ id: 'speaker', name: '小爱 Pro' }],
  selected_device_id: 'speaker',
  device_connection: 'connected',
  current_track_id: 'song',
  transport_state: 'PLAYING',
  position: 10,
  duration: 180,
  volume: 30,
  queue_mode: 'sequential',
};

function setup({ control = vi.fn(async () => undefined), state = initial } = {}) {
  const props = {
    state,
    ready: true,
    available: true,
    busy: false,
    reason: '',
    control,
    playTrack: vi.fn(),
  };
  const view = render(<Player {...props} />);
  return {
    ...view,
    control,
    update(next) {
      view.rerender(<Player {...props} state={{ ...state, ...next }} />);
    },
  };
}

describe('React player range reconciliation', () => {
  it('preserves focused volume and reconciles a polled external change on blur without replay', () => {
    const ui = setup();
    const volume = screen.getByRole('slider', { name: '音量' });
    fireEvent.focus(volume);
    ui.update({ volume: 65 });
    expect(volume.value).toBe('30');
    expect(screen.getByText('30', { selector: 'output' })).toBeTruthy();
    fireEvent.blur(volume);
    expect(volume.value).toBe('65');
    expect(screen.getByText('65', { selector: 'output' })).toBeTruthy();
    expect(ui.control).not.toHaveBeenCalled();
  });

  it('preserves a focused seek position and reconciles polling after focus ends', () => {
    const ui = setup();
    const seek = screen.getByRole('slider', { name: '播放进度' });
    fireEvent.focus(seek);
    ui.update({ position: 22 });
    expect(seek.value).toBe('10');
    expect(document.getElementById('elapsed').textContent).toBe('0:10');
    fireEvent.blur(seek);
    expect(seek.value).toBe('22');
    expect(document.getElementById('elapsed').textContent).toBe('0:22');
    expect(ui.control).not.toHaveBeenCalled();
  });

  it('restores the last backend value when a failed command settles, without automatic retry', async () => {
    let rejectCommand;
    const errors = [];
    const command = vi.fn(
      () =>
        new Promise((_resolve, reject) => {
          rejectCommand = reject;
        }),
    );
    // The application control boundary owns persistent error reporting. Player
    // receives its settled result, matching App's catch-without-replay contract.
    const control = vi.fn(async (...args) => {
      try {
        await command(...args);
      } catch (error) {
        errors.push(error.message);
      }
    });
    setup({ control });
    const volume = screen.getByRole('slider', { name: '音量' });
    fireEvent.focus(volume);
    fireEvent.change(volume, { target: { value: '75' } });
    fireEvent.pointerUp(volume);
    expect(control).toHaveBeenCalledExactlyOnceWith('volume', 75);
    fireEvent.blur(volume);
    await act(async () => {
      rejectCommand(new Error('音响暂时不可用'));
    });
    expect(errors).toEqual(['音响暂时不可用']);
    expect(volume.value).toBe('30');
    expect(screen.getByText('30', { selector: 'output' })).toBeTruthy();
    expect(command).toHaveBeenCalledTimes(1);
    expect(control).toHaveBeenCalledTimes(1);
  });

  it('commits a pointer adjustment once across pointer release, blur and command settlement', async () => {
    let finish;
    const control = vi.fn(
      () =>
        new Promise((resolve) => {
          finish = resolve;
        }),
    );
    setup({ control });
    const seek = screen.getByRole('slider', { name: '播放进度' });
    fireEvent.focus(seek);
    fireEvent.change(seek, { target: { value: '45' } });
    expect(control).not.toHaveBeenCalled();
    fireEvent.pointerUp(seek);
    fireEvent.pointerUp(seek);
    fireEvent.blur(seek);
    await act(async () => {
      finish();
    });
    expect(control).toHaveBeenCalledExactlyOnceWith('seek', 45);
  });

  it('commits a keyboard adjustment once and ignores unrelated keys', async () => {
    const ui = setup();
    const volume = screen.getByRole('slider', { name: '音量' });
    fireEvent.focus(volume);
    fireEvent.change(volume, { target: { value: '31' } });
    fireEvent.keyUp(volume, { key: 'a' });
    expect(ui.control).not.toHaveBeenCalled();
    await act(async () => {
      fireEvent.keyUp(volume, { key: 'ArrowRight' });
    });
    fireEvent.keyUp(volume, { key: 'ArrowRight' });
    fireEvent.blur(volume);
    expect(ui.control).toHaveBeenCalledExactlyOnceWith('volume', 31);
  });
});
