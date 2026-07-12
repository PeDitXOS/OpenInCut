import { useEffect, useState } from "react";

import type { TranscriptWord } from "../engine/types";
import {
  activeSequence,
  assetName,
  wordLabel,
  wordTimelineRange,
  wordsToCutRanges,
} from "../engine/types";
import { useStore } from "../state/store";

/**
 * Text-based editing: click a word to mark it (and jump there), then cut or
 * move the marked words — the video follows the text.
 */
export function TranscriptPanel() {
  const project = useStore((s) => s.project);
  const selection = useStore((s) => s.selection);
  const [assetSel, setAssetSel] = useState<string | null>(null);

  const transcripts = project.transcripts;

  // Selecting a clip on the timeline shows its transcript, when it has one.
  useEffect(() => {
    for (const id of selection) {
      for (const track of activeSequence(project).tracks) {
        const clip = track.clips.find((c) => c.id === id);
        if (!clip || clip.payload.type !== "media") continue;
        const assetId = clip.payload.asset_id;
        if (transcripts.some((t) => t.asset_id === assetId)) {
          setAssetSel(assetId);
          return;
        }
      }
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selection]);
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
      </div>
      <ReplaceBar transcriptId={doc.id} />
      <WordsView docId={doc.id} />
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
  // ⌘X stashes the marked block's timeline range; ⌘V moves it to the playhead
  const [moveStash, setMoveStash] = useState<[number, number] | null>(null);

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

  const stashForMove = () => {
    const words = [...selected].sort((a, b) => a - b).map((i) => doc.words[i]);
    const ranges = wordsToCutRanges(project, doc.asset_id, words, 0, 150_000);
    if (ranges.length !== 1) {
      useStore.setState({
        lastActionLabel: "⚠ to move, mark contiguous words (a single block)",
      });
      return;
    }
    setMoveStash(ranges[0]);
    setSelected(new Set());
    useStore.setState({
      lastActionLabel: `✂ ${words.length} word(s) ready to move — place the playhead and ⌘V`,
    });
  };

  const pasteStash = async () => {
    if (!moveStash) return;
    await moveTimelineRange(moveStash[0], moveStash[1], playheadUs);
    setMoveStash(null);
  };

  return (
    <>
      <div
        tabIndex={0}
        className="min-h-0 flex-1 select-text overflow-y-auto px-3 pb-2 text-[13px] leading-[1.9] outline-none"
        onKeyDown={(e) => {
          if ((e.target as HTMLElement).tagName === "INPUT") return; // WordEditor
          const mod = e.metaKey || e.ctrlKey;
          if ((e.key === "Backspace" || e.key === "Delete") && selected.size > 0) {
            e.preventDefault();
            e.stopPropagation(); // the global Backspace deletes timeline clips
            void cutSelected();
          } else if (mod && e.key.toLowerCase() === "x" && selected.size > 0) {
            e.preventDefault();
            e.stopPropagation();
            stashForMove();
          } else if (mod && e.key.toLowerCase() === "v" && moveStash) {
            e.preventDefault();
            e.stopPropagation();
            void pasteStash();
          } else if (e.key === "Escape") {
            setSelected(new Set());
            setMoveStash(null);
          }
        }}
      >
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
          Click marks the word and jumps to it · double-click renames · <b>Backspace</b> cuts the
          marked words · <b>⌘X</b>, place the playhead, <b>⌘V</b> moves them · <b>Esc</b> clears
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
      onClick={() => {
        onToggle();
        onSeek();
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
