import type { RealtimeChannel, RealtimeEvent, RealtimeStatus } from "./types";

export interface TranscriptEntry {
  id: string;
  channel: RealtimeChannel;
  segmentId: string;
  inputText: string;
  outputText: string;
  inputFinal: boolean;
  outputFinal: boolean;
  createdAt: number;
}

export interface ChannelRuntime {
  status: RealtimeStatus;
  level: number;
  error: string | null;
}

export interface RealtimeSessionState {
  channels: Record<RealtimeChannel, ChannelRuntime>;
  transcript: TranscriptEntry[];
}

export interface TranscriptExportLabels {
  microphone: string;
  system: string;
  translation: string;
}

export type SessionAction =
  | { type: "event"; event: RealtimeEvent; receivedAt?: number }
  | { type: "starting"; channel: RealtimeChannel }
  | { type: "stopping"; channel: RealtimeChannel }
  | { type: "clear-transcript" };

const idleChannel = (): ChannelRuntime => ({
  status: "idle",
  level: 0,
  error: null,
});

export function initialRealtimeSession(): RealtimeSessionState {
  return {
    channels: { microphone: idleChannel(), system: idleChannel() },
    transcript: [],
  };
}

export function realtimeSessionReducer(
  state: RealtimeSessionState,
  action: SessionAction,
): RealtimeSessionState {
  if (action.type === "clear-transcript") {
    return { ...state, transcript: [] };
  }
  if (action.type === "starting" || action.type === "stopping") {
    return updateChannel(state, action.channel, {
      status: action.type === "starting" ? "starting" : "stopping",
      error: null,
      ...(action.type === "stopping" ? { level: 0 } : {}),
    });
  }

  const event = action.event;
  if (event.kind === "status") {
    return updateChannel(state, event.channel, {
      status: event.status ?? "idle",
      level: event.status === "idle" ? 0 : state.channels[event.channel].level,
      error:
        event.status === "error"
          ? (event.message ?? state.channels[event.channel].error)
          : null,
    });
  }
  if (event.kind === "level") {
    return updateChannel(state, event.channel, {
      level: clampLevel(event.level),
    });
  }
  if (event.kind === "error") {
    return updateChannel(state, event.channel, {
      status: "error",
      level: 0,
      error: event.message?.trim() || "Realtime audio failed.",
    });
  }
  if (!event.segmentId) return state;

  const id = `${event.channel}:${event.segmentId}`;
  const index = state.transcript.findIndex((entry) => entry.id === id);
  const current: TranscriptEntry =
    index >= 0
      ? state.transcript[index]!
      : {
          id,
          channel: event.channel,
          segmentId: event.segmentId,
          inputText: "",
          outputText: "",
          inputFinal: false,
          outputFinal: false,
          createdAt: action.receivedAt ?? Date.now(),
        };
  const next = applyTranscriptEvent(current, event);
  if (next === current) return state;
  const transcript = [...state.transcript];
  if (index >= 0) transcript[index] = next;
  else transcript.push(next);
  return { ...state, transcript };
}

export function isChannelActive(runtime: ChannelRuntime): boolean {
  return (
    runtime.status === "starting" ||
    runtime.status === "listening" ||
    runtime.status === "stopping"
  );
}

export function transcriptAsText(
  entries: readonly TranscriptEntry[],
  labels: TranscriptExportLabels,
): string {
  return entries
    .filter((entry) => entry.inputText.trim() || entry.outputText.trim())
    .map((entry) => {
      const speaker =
        entry.channel === "microphone" ? labels.microphone : labels.system;
      const lines = [`${speaker}: ${entry.inputText.trim()}`];
      if (entry.outputText.trim()) {
        lines.push(`${labels.translation}: ${entry.outputText.trim()}`);
      }
      return lines.join("\n");
    })
    .join("\n\n");
}

function applyTranscriptEvent(
  entry: TranscriptEntry,
  event: RealtimeEvent,
): TranscriptEntry {
  switch (event.kind) {
    case "input_transcript_delta":
      return { ...entry, inputText: entry.inputText + (event.delta ?? "") };
    case "input_transcript_completed":
      return {
        ...entry,
        inputText: event.text ?? entry.inputText + (event.delta ?? ""),
        inputFinal: true,
      };
    case "output_transcript_delta":
      return { ...entry, outputText: entry.outputText + (event.delta ?? "") };
    case "output_transcript_completed":
      return {
        ...entry,
        outputText: event.text ?? entry.outputText + (event.delta ?? ""),
        outputFinal: true,
      };
    default:
      return entry;
  }
}

function updateChannel(
  state: RealtimeSessionState,
  channel: RealtimeChannel,
  patch: Partial<ChannelRuntime>,
): RealtimeSessionState {
  return {
    ...state,
    channels: {
      ...state.channels,
      [channel]: { ...state.channels[channel], ...patch },
    },
  };
}

function clampLevel(value: number | undefined): number {
  if (typeof value !== "number" || !Number.isFinite(value)) return 0;
  return Math.max(0, Math.min(1, value));
}
