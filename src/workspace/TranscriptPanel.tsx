import {
  Badge,
  CopyIconButton,
  EmptyState,
  Flex,
  IconButton,
  ScrollArea,
  Stack,
  Text,
  Tooltip,
} from "@orcestr/ui";
import { useEffect, useMemo, useRef } from "react";
import { LuMessageSquareText, LuTrash2 } from "react-icons/lu";

import { useAppI18n } from "../i18n/I18nProvider";
import { transcriptAsText, type TranscriptEntry } from "../realtime/session";
import { InfoHint } from "./InfoHint";

interface TranscriptPanelProps {
  entries: readonly TranscriptEntry[];
  onClear: () => void;
}

export function TranscriptPanel({ entries, onClear }: TranscriptPanelProps) {
  const { app, locale } = useAppI18n();
  const scrollRef = useRef<HTMLDivElement>(null);
  const copyText = useMemo(
    () =>
      transcriptAsText(entries, {
        microphone: app.transcript.exportYou,
        system: app.transcript.exportSystem,
        translation: app.transcript.exportTranslation,
      }),
    [app.transcript, entries],
  );

  useEffect(() => {
    const viewport = scrollRef.current;
    if (!viewport) return;
    viewport.scrollTo({ top: viewport.scrollHeight, behavior: "smooth" });
  }, [entries]);

  return (
    <section className="transcript-panel" aria-labelledby="transcript-title">
      <Flex a="c" g={2} className="transcript-toolbar">
        <LuMessageSquareText size={17} aria-hidden="true" />
        <Text id="transcript-title" fw={760}>
          {app.transcript.title}
        </Text>
        <InfoHint label={app.transcript.about} content={app.transcript.help} />
        <Badge v="soft" size={1} tone="neutral">
          {entries.length}
        </Badge>
        <span className="toolbar-spacer" />
        <Tooltip content={app.transcript.copy}>
          <CopyIconButton
            text={copyText}
            label={app.transcript.copy}
            successMessage={app.transcript.copied}
            v="ghost"
            size={2}
            disabled={!copyText}
          />
        </Tooltip>
        <Tooltip content={app.transcript.clear}>
          <IconButton
            icon={<LuTrash2 size={15} />}
            aria-label={app.transcript.clear}
            v="ghost"
            tone="neutral"
            size={2}
            disabled={entries.length === 0}
            onClick={onClear}
          />
        </Tooltip>
      </Flex>

      <ScrollArea
        ref={scrollRef}
        className="transcript-scroll"
        scrollbars="vertical"
        type="auto"
        highlights
        highlightH={30}
        highlightColor="#070707"
      >
        {entries.length === 0 ? (
          <EmptyState
            className="transcript-empty"
            v="ghost"
            compact
            icon={<LuMessageSquareText size={18} />}
            title={app.transcript.empty}
          />
        ) : (
          <Stack g={1} className="transcript-list" aria-live="polite">
            {entries.map((entry) => (
              <TranscriptRow key={entry.id} entry={entry} locale={locale} />
            ))}
          </Stack>
        )}
      </ScrollArea>
    </section>
  );
}

function TranscriptRow({
  entry,
  locale,
}: {
  entry: TranscriptEntry;
  locale: string;
}) {
  const { app } = useAppI18n();
  const partial = !entry.inputFinal || (entry.outputText && !entry.outputFinal);
  return (
    <article className={`transcript-row${partial ? " is-partial" : ""}`}>
      <Flex a="c" g={2} className="transcript-meta">
        <Badge
          size={1}
          v="soft"
          tone={entry.channel === "microphone" ? "primary" : "neutral"}
        >
          {entry.channel === "microphone"
            ? app.transcript.you
            : app.transcript.system}
        </Badge>
        <Text fs="10px" tone="muted">
          {formatTime(entry.createdAt, locale)}
        </Text>
        {partial ? (
          <Text fs="10px" tone="primary" className="live-word">
            {app.transcript.live}
          </Text>
        ) : null}
      </Flex>
      {entry.inputText ? (
        <Text className="transcript-original">{entry.inputText}</Text>
      ) : null}
      {entry.outputText ? (
        <Text className="transcript-translation" tone="primary">
          {entry.outputText}
        </Text>
      ) : null}
    </article>
  );
}

function formatTime(timestamp: number, locale: string): string {
  return new Intl.DateTimeFormat(locale, {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  }).format(timestamp);
}
