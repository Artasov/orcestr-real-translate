import { IconButton, Popover, Tooltip } from "@orcestr/ui";
import {
  useEffect,
  useId,
  useRef,
  useState,
  type CSSProperties,
  type FocusEvent,
} from "react";
import { LuVolume2, LuVolumeX } from "react-icons/lu";

import {
  PLAYBACK_VOLUME_MAX_DB,
  PLAYBACK_VOLUME_MIN_DB,
} from "../realtime/settings";

interface PlaybackVolumeControlProps {
  playbackOn: boolean;
  playbackPending: boolean;
  disabled: boolean;
  volumeDb: number;
  tooltip: string;
  toggleLabel: string;
  volumeLabel: string;
  onPlaybackChange: (enabled: boolean) => void;
  onVolumeChange: (volumeDb: number) => void;
  testId: string;
}

const CLOSE_DELAY_MS = 140;
const NEUTRAL_POSITION =
  ((0 - PLAYBACK_VOLUME_MIN_DB) /
    (PLAYBACK_VOLUME_MAX_DB - PLAYBACK_VOLUME_MIN_DB)) *
  100;

export function PlaybackVolumeControl({
  playbackOn,
  playbackPending,
  disabled,
  volumeDb,
  tooltip,
  toggleLabel,
  volumeLabel,
  onPlaybackChange,
  onVolumeChange,
  testId,
}: PlaybackVolumeControlProps) {
  const [open, setOpen] = useState(false);
  const closeTimer = useRef<number | null>(null);
  const dragging = useRef(false);
  const triggerHovered = useRef(false);
  const panelHovered = useRef(false);
  const labelId = useId();
  const position =
    ((volumeDb - PLAYBACK_VOLUME_MIN_DB) /
      (PLAYBACK_VOLUME_MAX_DB - PLAYBACK_VOLUME_MIN_DB)) *
    100;
  const rangeStart = Math.min(position, NEUTRAL_POSITION);
  const rangeEnd = Math.max(position, NEUTRAL_POSITION);
  const sliderStyle = {
    width: 184,
    minWidth: 184,
    "--volume-range-start": `${rangeStart}%`,
    "--volume-range-end": `${rangeEnd}%`,
    "--volume-neutral-position": `${NEUTRAL_POSITION}%`,
  } as CSSProperties;

  const clearCloseTimer = () => {
    if (closeTimer.current === null) return;
    window.clearTimeout(closeTimer.current);
    closeTimer.current = null;
  };

  const showPanel = () => {
    clearCloseTimer();
    if (!disabled) setOpen(true);
  };

  const scheduleClose = () => {
    clearCloseTimer();
    if (dragging.current) return;
    closeTimer.current = window.setTimeout(() => {
      closeTimer.current = null;
      if (
        dragging.current ||
        triggerHovered.current ||
        panelHovered.current
      ) {
        return;
      }
      setOpen(false);
    }, CLOSE_DELAY_MS);
  };

  const finishDragging = () => {
    if (!dragging.current) return;
    dragging.current = false;
    if (!triggerHovered.current && !panelHovered.current) scheduleClose();
  };

  useEffect(
    () => {
      window.addEventListener("pointerup", finishDragging);
      window.addEventListener("pointercancel", finishDragging);
      return () => {
        window.removeEventListener("pointerup", finishDragging);
        window.removeEventListener("pointercancel", finishDragging);
        if (closeTimer.current !== null) window.clearTimeout(closeTimer.current);
      };
    },
    [],
  );

  useEffect(() => {
    if (disabled) setOpen(false);
  }, [disabled]);

  const handleTriggerEnter = () => {
    triggerHovered.current = true;
    showPanel();
  };

  const handleTriggerLeave = () => {
    triggerHovered.current = false;
    scheduleClose();
  };

  const handlePanelEnter = () => {
    panelHovered.current = true;
    showPanel();
  };

  const handlePanelLeave = () => {
    panelHovered.current = false;
    scheduleClose();
  };

  const handleBlur = (event: FocusEvent<HTMLDivElement>) => {
    if (!event.currentTarget.contains(event.relatedTarget)) scheduleClose();
  };

  return (
    <Popover
      open={open}
      onOpenChange={(nextOpen) => {
        if (nextOpen) showPanel();
        else if (!dragging.current) setOpen(false);
      }}
      side="bottom"
      align="end"
      sideOffset={5}
      collisionPadding={10}
      disabled={disabled}
      className="playback-volume-popover"
      contentStyle={sliderStyle}
      testId={`${testId}-volume-panel`}
      onOpenAutoFocus={(event) => event.preventDefault()}
      onMouseEnter={handlePanelEnter}
      onMouseLeave={handlePanelLeave}
      onFocusCapture={showPanel}
      onBlurCapture={handleBlur}
      trigger={
        <div
          className="playback-volume-control"
          onClick={(event) => event.preventDefault()}
          onMouseEnter={handleTriggerEnter}
          onMouseLeave={handleTriggerLeave}
          onFocusCapture={showPanel}
          onBlurCapture={handleBlur}
        >
          <Tooltip content={tooltip} side="top">
            <IconButton
              icon={
                playbackOn ? <LuVolume2 size={16} /> : <LuVolumeX size={16} />
              }
              aria-label={toggleLabel}
              aria-pressed={playbackOn}
              aria-haspopup="true"
              aria-expanded={open}
              tone={playbackOn ? "primary" : "neutral"}
              v={playbackOn ? "soft" : "ghost"}
              size={2}
              className="channel-playback-toggle"
              testId={testId}
              loading={playbackPending}
              disabled={disabled || playbackPending}
              onClick={() => onPlaybackChange(!playbackOn)}
            />
          </Tooltip>
        </div>
      }
    >
      <div className="playback-volume-row" role="group" aria-labelledby={labelId}>
        <span id={labelId} className="playback-volume-label">
          {volumeLabel}
        </span>
        <div className="playback-volume-slider-shell">
          <span className="playback-volume-neutral" aria-hidden="true" />
          <input
            id={`${testId}-volume`}
            className="playback-volume-slider"
            type="range"
            dir="ltr"
            min={PLAYBACK_VOLUME_MIN_DB}
            max={PLAYBACK_VOLUME_MAX_DB}
            step={1}
            value={volumeDb}
            aria-label={volumeLabel}
            aria-valuetext={formatVolumeDb(volumeDb)}
            onPointerDown={() => {
              dragging.current = true;
              clearCloseTimer();
            }}
            onChange={(event) =>
              onVolumeChange(Number(event.currentTarget.value))
            }
          />
        </div>
        <output
          className="playback-volume-value"
          htmlFor={`${testId}-volume`}
        >
          {formatVolumeDb(volumeDb)}
        </output>
      </div>
    </Popover>
  );
}

function formatVolumeDb(volumeDb: number): string {
  if (volumeDb > 0) return `+${volumeDb} dB`;
  return `${volumeDb} dB`;
}
