import { useEffect } from "react";

import { useStore } from "../state/store";
import { frameToUs } from "../lib/time";
import { Header } from "./Header";
import { MediaPool } from "./MediaPool";
import { Preview } from "./Preview";
import { Inspector } from "./Inspector";
import { Timeline } from "./Timeline";
import { StatusBar } from "./StatusBar";

function useKeyboard() {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const s = useStore.getState();
      const mod = e.metaKey || e.ctrlKey;
      const inInput =
        (e.target as HTMLElement)?.tagName === "INPUT" ||
        (e.target as HTMLElement)?.tagName === "TEXTAREA";
      if (inInput) return;

      if (e.code === "Space") {
        e.preventDefault();
        s.togglePlay();
      } else if (mod && e.key.toLowerCase() === "z" && e.shiftKey) {
        e.preventDefault();
        s.redo();
      } else if (mod && e.key.toLowerCase() === "z") {
        e.preventDefault();
        s.undo();
      } else if (e.key.toLowerCase() === "k" && mod) {
        e.preventDefault();
        s.splitAtPlayhead();
      } else if (e.key.toLowerCase() === "s" && !mod) {
        s.splitAtPlayhead();
      } else if (e.key === "Delete" || e.key === "Backspace") {
        e.preventDefault();
        s.deleteSelection(e.shiftKey);
      } else if (e.key === "ArrowLeft" || e.key === "ArrowRight") {
        e.preventDefault();
        const fps = s.project.sequences[0].fps;
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
  useEffect(() => {
    if (!playing) return;
    let raf = 0;
    let last = performance.now();
    const tick = (now: number) => {
      const dt = now - last;
      last = now;
      const s = useStore.getState();
      s.seek(s.playheadUs + dt * 1000);
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [playing]);
}

export function App() {
  useKeyboard();
  usePlayback();

  return (
    <div className="flex h-full flex-col bg-bg0">
      <Header />
      <main className="flex min-h-0 flex-1">
        <aside className="w-[264px] shrink-0 border-r border-line-soft bg-bg1">
          <MediaPool />
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
