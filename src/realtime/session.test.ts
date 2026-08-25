import { describe, expect, it } from "vitest";

import {
  initialRealtimeSession,
  realtimeSessionReducer,
  transcriptAsText,
} from "./session";

describe("realtime session reducer", () => {
  it("merges input and output deltas into one segment and finalizes exact text", () => {
    let state = initialRealtimeSession();
    state = realtimeSessionReducer(state, {
      type: "event",
      receivedAt: 100,
      event: {
        channel: "microphone",
        kind: "input_transcript_delta",
        segmentId: "turn-1",
        delta: "Hel",
      },
    });
    state = realtimeSessionReducer(state, {
      type: "event",
      event: {
        channel: "microphone",
        kind: "input_transcript_delta",
        segmentId: "turn-1",
        delta: "lo",
      },
    });
    state = realtimeSessionReducer(state, {
      type: "event",
      event: {
        channel: "microphone",
        kind: "output_transcript_delta",
        segmentId: "turn-1",
        delta: "При",
      },
    });
    state = realtimeSessionReducer(state, {
      type: "event",
      event: {
        channel: "microphone",
        kind: "input_transcript_completed",
        segmentId: "turn-1",
        text: "Hello",
      },
    });
    state = realtimeSessionReducer(state, {
      type: "event",
      event: {
        channel: "microphone",
        kind: "output_transcript_completed",
        segmentId: "turn-1",
        text: "Привет",
      },
    });

    expect(state.transcript).toEqual([
      expect.objectContaining({
        id: "microphone:turn-1",
        createdAt: 100,
        inputText: "Hello",
        outputText: "Привет",
        inputFinal: true,
        outputFinal: true,
      }),
    ]);
  });

  it("handles translated output arriving before the input transcript", () => {
    let state = initialRealtimeSession();
    state = realtimeSessionReducer(state, {
      type: "event",
      event: {
        channel: "system",
        kind: "output_transcript_completed",
        segmentId: "remote-1",
        text: "How are you?",
      },
    });
    state = realtimeSessionReducer(state, {
      type: "event",
      event: {
        channel: "system",
        kind: "input_transcript_completed",
        segmentId: "remote-1",
        text: "Как дела?",
      },
    });

    expect(state.transcript).toHaveLength(1);
    expect(state.transcript[0]).toMatchObject({
      channel: "system",
      inputText: "Как дела?",
      outputText: "How are you?",
    });
  });

  it("keeps equal segment ids from different channels separate", () => {
    let state = initialRealtimeSession();
    for (const channel of ["microphone", "system"] as const) {
      state = realtimeSessionReducer(state, {
        type: "event",
        event: {
          channel,
          kind: "input_transcript_completed",
          segmentId: "1",
          text: channel,
        },
      });
    }
    expect(state.transcript.map((entry) => entry.id)).toEqual([
      "microphone:1",
      "system:1",
    ]);
  });

  it("clears only transcript while preserving channel runtime", () => {
    let state = initialRealtimeSession();
    state = realtimeSessionReducer(state, {
      type: "event",
      event: {
        channel: "microphone",
        kind: "status",
        status: "listening",
      },
    });
    state = realtimeSessionReducer(state, {
      type: "event",
      event: {
        channel: "microphone",
        kind: "input_transcript_completed",
        segmentId: "1",
        text: "Keep listening",
      },
    });
    state = realtimeSessionReducer(state, { type: "clear-transcript" });

    expect(state.transcript).toEqual([]);
    expect(state.channels.microphone.status).toBe("listening");
  });

  it("clamps audio level and formats a copyable transcript", () => {
    let state = initialRealtimeSession();
    state = realtimeSessionReducer(state, {
      type: "event",
      event: { channel: "system", kind: "level", level: 9 },
    });
    state = realtimeSessionReducer(state, {
      type: "event",
      event: {
        channel: "system",
        kind: "input_transcript_completed",
        segmentId: "1",
        text: "Bonjour",
      },
    });
    state = realtimeSessionReducer(state, {
      type: "event",
      event: {
        channel: "system",
        kind: "output_transcript_completed",
        segmentId: "1",
        text: "Hello",
      },
    });

    expect(state.channels.system.level).toBe(1);
    expect(
      transcriptAsText(state.transcript, {
        microphone: "You",
        system: "System",
        translation: "Translation",
      }),
    ).toBe("System: Bonjour\nTranslation: Hello");
  });
});
