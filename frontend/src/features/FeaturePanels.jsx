import React, { useEffect } from 'react';
import { createPortal } from 'react-dom';
import useFeatureController from './useFeatureController.js';
import BilibiliPanel from './BilibiliPanel.jsx';
import VoicePanel from './VoicePanel.jsx';
import MobilePanel from './MobilePanel.jsx';
import DownloadQueue from './DownloadQueue.jsx';

export default function FeaturePanels({
  panel,
  onClose,
  invoke,
  connected,
  refresh,
  announce,
  onVoiceBadge,
  queueTarget,
}) {
  const controller = useFeatureController({
    invoke,
    connected,
    refresh,
    announce,
    onVoiceBadge,
  });
  useEffect(() => {
    if (panel) controller.poll();
  }, [panel, controller.poll]);
  return (
    <>
      <BilibiliPanel open={panel === 'bilibili'} onClose={onClose} controller={controller} />
      <VoicePanel open={panel === 'voice'} onClose={onClose} controller={controller} />
      <MobilePanel open={panel === 'mobile'} onClose={onClose} controller={controller} />
      {queueTarget && createPortal(<DownloadQueue controller={controller} />, queueTarget)}
    </>
  );
}
