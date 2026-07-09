import { useEffect } from "react";

import { activeSequence } from "../engine/types";
import { frameToUs } from "../lib/time";
import { engine, useStore } from "../state/store";
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
      const el = e.target as HTMLElement;
      if (el?.tagName === "INPUT" || el?.tagName === "TEXTAREA") return;

      if (e.code === "Space") {
        e.preventDefault();
        s.togglePlay();
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
          const [t, isPlaying] = await engine.playbackPosition();
          useStore.setState({ playheadUs: t });
          if (!isPlaying) useStore.setState({ playing: false });
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

export function App() {
  const init = useStore((s) => s.init);
  useEffect(() => {
    void init();
  }, [init]);
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
