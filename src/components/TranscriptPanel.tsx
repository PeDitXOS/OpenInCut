import { useMemo, useState } from "react";

import type { TranscriptWord } from "../engine/types";
import {
  assetName,
  timelineDocument,
  tokenRunRange,
  wordLabel,
  wordTimelineRange,
  wordsToCutRanges,
} from "../engine/types";
import { useStore } from "../state/store";

/**
 * Text-based editing. Two views:
 * - Words: mark words → cut/move (click marks, double-click renames).
 * - Document: the timeline projected as text; select and Backspace deletes,
 *   ⌘X + ⌘V moves material — the video follows the text.
 */
export function TranscriptPanel() {
  const project = useStore((s) => s.project);
  const [assetSel, setAssetSel] = useState<string | null>(null);
  const [view, setView] = useState<"words" | "document">("words");

  const transcripts = project.transcripts;
  if (!transcripts.length) {
    return (
      <div className="px-3 py-4 text-[11px] leading-relaxed text-ink-faint">
        <p className="mb-2 font-medium text-ink-dim">No transcripts yet.</p>
        <p>
          Select a clip and press <span className="text-accent">🎙 Transcribe (Whisper)</span> in
          the Inspector (or the button on the media item).
        </p>
        <p className="mt-2">Then you can edit the video by deleting or moving text.</p>
      </div>
    );
  }

  const doc = transcripts.find((t) => t.asset_id === assetSel) ?? transcripts[0];

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex items-center gap-1.5 px-3 pb-2 pt-1">
        <select
          className="focus-ring min-w-0 flex-1 cursor-pointer rounded-md border border-line bg-bg2 px-2 py-1 text-[11px] text-ink"
          value={doc.asset_id}
          onChange={(e) => setAssetSel(e.target.value)}
          title="Transcript to show"
        >
          {transcripts.map((t) => (
            <option key={t.id} value={t.asset_id}>
              {assetName(project.assets.find((a) => a.id === t.asset_id))}
            </option>
          ))}
        </select>
        {(["words", "document"] as const).map((v) => (
          <button
            key={v}
            className={`focus-ring rounded-md px-2 py-1 text-[11px] ${
              view === v ? "bg-bg3 text-ink" : "text-ink-faint hover:text-ink"
            }`}
            onClick={() => setView(v)}
            title={
              v === "words"
                ? "Mark words to cut or move them"
                : "Edit the text like a document: the video follows"
            }
          >
            {v === "words" ? "Words" : "Document"}
          </button>
        ))}
      </div>
      <ReplaceBar transcriptId={doc.id} />
      {view === "words" ? <WordsView docId={doc.id} /> : <DocumentView docId={doc.id} />}
    </div>
  );
}

/** Fix transcription errors everywhere: godo → godot. */
function ReplaceBar({ transcriptId }: { transcriptId: string }) {
  const replaceWords = useStore((s) => s.replaceWords);
  const [from, setFrom] = useState("");
  const [to, setTo] = useState("");
  const inputCls =
    "focus-ring w-0 min-w-0 flex-1 rounded-md border border-line bg-bg2 px-2 py-1 text-[11px] text-ink placeholder:text-ink-faint";
  return (
    <div className="flex items-center gap-1 px-3 pb-2">
      <input
        className={inputCls}
        placeholder="godo"
        value={from}
        onChange={(e) => setFrom(e.target.value)}
        title="Word as it was transcribed"
      />
      <span className="text-[10px] text-ink-faint">→</span>
      <input
        className={inputCls}
        placeholder="godot"
        value={to}
        onChange={(e) => setTo(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && from.trim()) {
            void replaceWords(transcriptId, from, to);
            setFrom("");
            setTo("");
          }
        }}
        title="Correction (audio timing is untouched; captions show this)"
      />
      <button
        className="focus-ring rounded-md border border-line px-2 py-1 text-[11px] text-ink-dim enabled:hover:text-ink disabled:opacity-40"
        disabled={!from.trim()}
        onClick={() => {
          void replaceWords(transcriptId, from, to);
          setFrom("");
          setTo("");
        }}
        title="Replace every occurrence (1 undo)"
      >
        Replace
      </button>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Words view: mark → cut / move; double-click renames
// ---------------------------------------------------------------------------

function WordsView({ docId }: { docId: string }) {
  const project = useStore((s) => s.project);
  const seek = useStore((s) => s.seek);
  const cutTimelineRanges = useStore((s) => s.cutTimelineRanges);
  const moveTimelineRange = useStore((s) => s.moveTimelineRange);
  const setWordText = useStore((s) => s.setWordText);
  const playheadUs = useStore((s) => s.playheadUs);
  const [selected, setSelected] = useState<Set<number>>(new Set());
  const [editing, setEditing] = useState<number | null>(null);

  const doc = project.transcripts.find((t) => t.id === docId);
  if (!doc) return null;
  const asset = project.assets.find((a) => a.id === doc.asset_id);

  const toggle = (i: number) =>
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(i)) next.delete(i);
      else next.add(i);
      return next;
    });

  const cutSelected = async () => {
    const words = [...selected].sort((a, b) => a - b).map((i) => doc.words[i]);
    const ranges = wordsToCutRanges(project, doc.asset_id, words);
    await cutTimelineRanges(ranges);
    setSelected(new Set());
  };

  const moveSelectedToPlayhead = async () => {
    const words = [...selected].sort((a, b) => a - b).map((i) => doc.words[i]);
    const ranges = wordsToCutRanges(project, doc.asset_id, words, 0, 150_000);
    if (ranges.length !== 1) {
      useStore.setState({
        lastActionLabel: "⚠ to move, select contiguous words (a single block)",
      });
      return;
    }
    await moveTimelineRange(ranges[0][0], ranges[0][1], playheadUs);
    setSelected(new Set());
  };

  return (
    <>
      <div className="min-h-0 flex-1 select-text overflow-y-auto px-3 pb-2 text-[13px] leading-[1.9]">
        <p className="mb-2">
          {doc.words.map((w, i) =>
            editing === i ? (
              <WordEditor
                key={i}
                word={w}
                onDone={(text) => {
                  setEditing(null);
                  if (text !== null) void setWordText(doc.id, i, text);
                }}
              />
            ) : (
              <WordSpan
                key={i}
                word={w}
                selected={selected.has(i)}
                underPlayhead={isUnderPlayhead(project, doc.asset_id, w, playheadUs)}
                onToggle={() => toggle(i)}
                onRename={() => setEditing(i)}
                onSeek={() => {
                  const r = wordTimelineRange(project, doc.asset_id, w);
                  if (r) seek(r[0]);
                }}
              />
            ),
          )}
        </p>
        <div className="mt-2 font-[var(--font-mono)] text-[10px] text-ink-faint">
          {doc.words.length} words · model {doc.model}
          {asset && ` · ${assetName(asset)}`}
        </div>
      </div>

      <div className="border-t border-line-soft p-2">
        <div className="flex gap-2">
          <button
            className="focus-ring flex-1 rounded-md bg-danger/80 px-2 py-1.5 text-[12px] font-medium text-white enabled:hover:bg-danger disabled:opacity-40"
            disabled={selected.size === 0}
            onClick={() => void cutSelected()}
            title="Cut the marked words from the video (closes gaps; 1 undo)"
          >
            ✂ Cut {selected.size > 0 ? `${selected.size} word(s)` : "selection"}
          </button>
          <button
            className="focus-ring rounded-md border border-line px-2 py-1.5 text-[12px] text-ink-dim enabled:hover:text-ink disabled:opacity-40"
            disabled={selected.size === 0}
            onClick={() => void moveSelectedToPlayhead()}
            title="Move the marked (contiguous) words to the playhead"
          >
            ⇢ Move
          </button>
          <button
            className="focus-ring rounded-md border border-line px-2 py-1.5 text-[12px] text-ink-dim enabled:hover:text-ink disabled:opacity-40"
            disabled={selected.size === 0}
            onClick={() => setSelected(new Set())}
          >
            Clear
          </button>
        </div>
        <p className="mt-1.5 text-[10px] leading-snug text-ink-faint">
          Click marks · double-click renames · ⌥click jumps to the word
        </p>
      </div>
    </>
  );
}

function isUnderPlayhead(
  project: ReturnType<typeof useStore.getState>["project"],
  assetId: string,
  word: TranscriptWord,
  playheadUs: number,
): boolean {
  const r = wordTimelineRange(project, assetId, word);
  return r !== null && playheadUs >= r[0] && playheadUs < r[1];
}

function WordSpan({
  word,
  selected,
  underPlayhead,
  onToggle,
  onRename,
  onSeek,
}: {
  word: TranscriptWord;
  selected: boolean;
  underPlayhead: boolean;
  onToggle: () => void;
  onRename: () => void;
  onSeek: () => void;
}) {
  return (
    <span
      className={[
        "cursor-pointer rounded px-0.5",
        selected
          ? "bg-danger/30 text-danger line-through"
          : underPlayhead
            ? "bg-accent/25 text-ink"
            : word.display
              ? "text-accent/90 hover:bg-bg3"
              : "text-ink-dim hover:bg-bg3 hover:text-ink",
      ].join(" ")}
      title={word.display ? `Corrected (was “${word.text}”)` : undefined}
      onClick={(e) => {
        if (e.altKey) onSeek();
        else onToggle();
      }}
      onDoubleClick={(e) => {
        e.preventDefault();
        onRename();
      }}
    >
      {wordLabel(word)}{" "}
    </span>
  );
}

/** Inline rename input: Enter commits, Esc cancels, empty reverts. */
function WordEditor({
  word,
  onDone,
}: {
  word: TranscriptWord;
  onDone: (text: string | null) => void;
}) {
  const [value, setValue] = useState(wordLabel(word));
  return (
    <input
      autoFocus
      className="focus-ring mx-0.5 inline-block w-24 rounded border border-accent bg-bg2 px-1 text-[12px] text-ink"
      value={value}
      onChange={(e) => setValue(e.target.value)}
      onBlur={() => onDone(value)}
      onKeyDown={(e) => {
        if (e.key === "Enter") onDone(value);
        if (e.key === "Escape") onDone(null);
      }}
    />
  );
}

// ---------------------------------------------------------------------------
// Document view: the timeline as text; editing it edits the video
// ---------------------------------------------------------------------------

function DocumentView({ docId }: { docId: string }) {
  const project = useStore((s) => s.project);
  const version = useStore((s) => s.version);
  const seek = useStore((s) => s.seek);
  const playheadUs = useStore((s) => s.playheadUs);
  const cutTimelineRanges = useStore((s) => s.cutTimelineRanges);
  const moveTimelineRange = useStore((s) => s.moveTimelineRange);
  const [anchor, setAnchor] = useState<number | null>(null);
  const [focusIdx, setFocusIdx] = useState<number | null>(null);
  const [clipboard, setClipboard] = useState<[number, number] | null>(null);

  const doc = project.transcripts.find((t) => t.id === docId);
  const tokens = useMemo(
    () => (doc ? timelineDocument(project, doc) : []),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [project, version, docId],
  );
  if (!doc) return null;

  const sel: [number, number] | null =
    anchor !== null && focusIdx !== null
      ? [Math.min(anchor, focusIdx), Math.max(anchor, focusIdx)]
      : null;

  /** Contiguous runs of the selection (deleting may span several clips). */
  const selectionRanges = (): [number, number][] => {
    if (!sel) return [];
    return [[sel[0], sel[1]]];
  };

  const clearSel = () => {
    setAnchor(null);
    setFocusIdx(null);
  };

  const deleteSelection = async () => {
    if (!sel) return;
    const ranges = selectionRanges().map(([a, b]) => tokenRunRange(tokens, a, b));
    await cutTimelineRanges(ranges);
    clearSel();
    setClipboard(null);
  };

  const cutToClipboard = () => {
    if (!sel) return;
    setClipboard(sel);
    useStore.setState({
      lastActionLabel: `✂ ${sel[1] - sel[0] + 1} word(s) on the clipboard — click a spot and ⌘V`,
    });
  };

  const pasteAtCaret = async () => {
    if (!clipboard || focusIdx === null) return;
    const [a, b] = clipboard;
    if (focusIdx >= a && focusIdx <= b) {
      useStore.setState({ lastActionLabel: "⚠ paste outside the cut block" });
      return;
    }
    const [from, to] = tokenRunRange(tokens, a, b);
    // paste BEFORE the caret token
    const dest = focusIdx > b ? tokens[focusIdx].tlStart : tokens[focusIdx].tlStart;
    await moveTimelineRange(from, to, dest);
    setClipboard(null);
    clearSel();
  };

  return (
    <>
      <div
        tabIndex={0}
        className="focus-ring m-1 min-h-0 flex-1 cursor-text select-none overflow-y-auto rounded-md px-2 pb-2 text-[13px] leading-[1.9] outline-none"
        onKeyDown={(e) => {
          const mod = e.metaKey || e.ctrlKey;
          if (e.key === "Backspace" || e.key === "Delete") {
            e.preventDefault();
            void deleteSelection();
          } else if (mod && e.key.toLowerCase() === "x") {
            e.preventDefault();
            cutToClipboard();
          } else if (mod && e.key.toLowerCase() === "v") {
            e.preventDefault();
            void pasteAtCaret();
          } else if (e.key === "Escape") {
            clearSel();
            setClipboard(null);
          }
        }}
      >
        {tokens.length === 0 && (
          <p className="pt-2 text-[11px] text-ink-faint">
            No material of this transcript on the timeline.
          </p>
        )}
        {tokens.map((t, i) => {
          const inSel = sel !== null && i >= sel[0] && i <= sel[1];
          const inClipboard = clipboard !== null && i >= clipboard[0] && i <= clipboard[1];
          const under = playheadUs >= t.tlStart && playheadUs < t.tlEnd;
          return (
            <span
              key={t.key}
              className={[
                "rounded px-0.5",
                inClipboard
                  ? "bg-accent/15 text-accent/70 italic"
                  : inSel
                    ? "bg-accent/30 text-ink"
                    : under
                      ? "bg-accent/20 text-ink"
                      : "text-ink-dim hover:bg-bg3 hover:text-ink",
              ].join(" ")}
              onClick={(e) => {
                if (e.shiftKey && anchor !== null) setFocusIdx(i);
                else {
                  setAnchor(i);
                  setFocusIdx(i);
                  seek(t.tlStart);
                }
              }}
            >
              {wordLabel(t.word)}{" "}
            </span>
          );
        })}
      </div>
      <div className="border-t border-line-soft p-2 text-[10px] leading-snug text-ink-faint">
        Click places the caret (and seeks) · ⇧click selects · <b>Backspace</b> deletes the
        selection from the video · <b>⌘X</b> then click + <b>⌘V</b> moves it — everything is one
        undo step.
      </div>
    </>
  );
}
