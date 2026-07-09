import { useEffect, useState } from "react";

import { activeSequence } from "../engine/types";
import { frameToUs } from "../lib/time";
import { engine, useStore } from "../state/store";
import { Header } from "./Header";
import { MediaPool } from "./MediaPool";
import { Preview } from "./Preview";
import { Inspector } from "./Inspector";
import { Timeline } from "./Timeline";
import { StatusBar } from "./StatusBar";
import { TranscriptPanel } from "./TranscriptPanel";

function useKeyboard() {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const s = useStore.getState();
      const mod = e.metaKey || e.ctrlKey;
      const el = e.target as HTMLElement;
      if (el?.tagName === "INPUT" || el?.tagName === "TEXTAREA") return;

      if (e.code === "Space") {
        e.preventDefault();
        s.togglePlay();
      } else if (mod && e.key.toLowerCase() === "s") {
        e.preventDefault();
        void s.saveProject();
      } else if (mod && e.key.toLowerCase() === "o") {
        e.preventDefault();
        void s.openProject();
      } else if (mod && e.key.toLowerCase() === "z" && e.shiftKey) {
        e.preventDefault();
        void s.redo();
      } else if (mod && e.key.toLowerCase() === "z") {
        e.preventDefault();
        void s.undo();
      } else if (e.key.toLowerCase() === "k" && mod) {
        e.preventDefault();
        void s.splitAtPlayhead();
      } else if (e.key.toLowerCase() === "s" && !mod) {
        void s.splitAtPlayhead();
      } else if (e.key === "Delete" || e.key === "Backspace") {
        e.preventDefault();
        void s.deleteSelection(e.shiftKey);
      } else if (e.key === "ArrowLeft" || e.key === "ArrowRight") {
        e.preventDefault();
        const fps = activeSequence(s.project).fps;
        const step = frameToUs(1, fps) * (e.shiftKey ? 10 : 1);
        s.seek(s.playheadUs + (e.key === "ArrowLeft" ? -step : step));
      } else if (e.key === "Home") {
        s.seek(0);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
}

function usePlayback() {
  const playing = useStore((s) => s.playing);
  const engineClock = useStore((s) => s.engineClock);
  useEffect(() => {
    if (!playing) return;

    if (engineClock) {
      // el reloj de audio del backend manda: sondear posición
      const id = window.setInterval(async () => {
        try {
          const [t, isPlaying, meterL, meterR] = await engine.playbackPosition();
          useStore.setState({ playheadUs: t, meterL, meterR });
          if (!isPlaying) useStore.setState({ playing: false, meterL: 0, meterR: 0 });
        } catch {
          useStore.setState({ engineClock: false });
        }
      }, 33);
      return () => window.clearInterval(id);
    }

    // reloj local (navegador / sin dispositivo de audio)
    let raf = 0;
    let last = performance.now();
    const tick = (now: number) => {
      const dt = now - last;
      last = now;
      const s = useStore.getState();
      useStore.setState({ playheadUs: s.playheadUs + dt * 1000 });
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [playing, engineClock]);
}

const MEDIA_EXTENSIONS = new Set([
  "mp4", "mov", "mkv", "webm", "avi", "m4v", "mts", "mpg",
  "wav", "mp3", "m4a", "aac", "flac", "ogg", "aiff",
  "png", "jpg", "jpeg", "webp", "bmp", "tiff", "gif",
]);

/** Drag & drop de archivos desde el sistema (solo escritorio). */
function useNativeFileDrop() {
  useEffect(() => {
    if (engine.kind !== "tauri") return;
    let unlisten: (() => void) | undefined;
    void (async () => {
      const { getCurrentWebview } = await import("@tauri-apps/api/webview");
      unlisten = await getCurrentWebview().onDragDropEvent((event) => {
        if (event.payload.type !== "drop") return;
        const paths = event.payload.paths.filter((p) => {
          const ext = p.split(".").pop()?.toLowerCase() ?? "";
          return MEDIA_EXTENSIONS.has(ext);
        });
        if (!paths.length) return;
        void (async () => {
          try {
            const snap = await engine.importMedia(paths);
            useStore.setState({
              project: snap.project,
              version: snap.version,
              dirty: snap.dirty,
              canUndo: snap.can_undo,
              canRedo: snap.can_redo,
              lastActionLabel: `Importados ${paths.length} archivo(s) arrastrados`,
            });
          } catch (e) {
            useStore.setState({
              lastActionLabel: `⚠ ${e instanceof Error ? e.message : String(e)}`,
            });
          }
        })();
      });
    })();
    return () => unlisten?.();
  }, []);
}

export function App() {
  const init = useStore((s) => s.init);
  const [leftTab, setLeftTab] = useState<"media" | "texto">("media");
  const transcriptCount = useStore((s) => s.project.transcripts.length);
  useEffect(() => {
    void init();
  }, [init]);
  useKeyboard();
  usePlayback();
  useNativeFileDrop();

  return (
    <div className="flex h-full flex-col bg-bg0">
      <Header />
      <main className="flex min-h-0 flex-1">
        <aside className="flex w-[264px] shrink-0 flex-col border-r border-line-soft bg-bg1">
          <div className="flex gap-1 px-2 pt-2">
            {(
              [
                ["media", "Medios"],
                ["texto", `Texto${transcriptCount ? ` (${transcriptCount})` : ""}`],
              ] as const
            ).map(([key, label]) => (
              <button
                key={key}
                className={`focus-ring rounded-md px-2.5 py-1 text-[11px] font-medium ${
                  leftTab === key ? "bg-bg3 text-ink" : "text-ink-faint hover:text-ink"
                }`}
                onClick={() => setLeftTab(key)}
              >
                {label}
              </button>
            ))}
          </div>
          <div className="min-h-0 flex-1">
            {leftTab === "media" ? <MediaPool /> : <TranscriptPanel />}
          </div>
        </aside>
        <section className="flex min-w-0 flex-1 flex-col bg-bg0">
          <Preview />
        </section>
        <aside className="w-[292px] shrink-0 border-l border-line-soft bg-bg1">
          <Inspector />
        </aside>
      </main>
      <section className="h-[292px] shrink-0 border-t border-line bg-bg1">
        <Timeline />
      </section>
      <StatusBar />
    </div>
  );
}
