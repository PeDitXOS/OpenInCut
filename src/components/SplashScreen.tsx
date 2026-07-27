import { useState, useEffect } from "react";

// Try to load logo; falls back to text if image not found
let logoSrc: string | null = null;
try {
  logoSrc = new URL("../assets/logo.png", import.meta.url).href;
} catch {
  /* no logo yet */
}

export function SplashScreen() {
  const [visible, setVisible] = useState(true);

  useEffect(() => {
    const t = setTimeout(() => setVisible(false), 2000);
    return () => clearTimeout(t);
  }, []);

  if (!visible) return null;

  return (
    <div
      className="fixed inset-0 z-[9999] flex flex-col items-center justify-center bg-bg0 transition-opacity duration-500"
      style={{ opacity: visible ? 1 : 0 }}
    >
      {logoSrc ? (
        <img
          src={logoSrc}
          alt="OpenInCut"
          className="mb-4 h-20 w-20 animate-pulse"
          onError={() => {
            // hide broken image — text-only fallback shows below
            (document.getElementById("splash-logo") as HTMLImageElement).style.display =
              "none";
          }}
        />
      ) : null}
      <h1 className="text-2xl font-bold tracking-wide text-ink animate-pulse">
        OpenInCut
      </h1>
    </div>
  );
}
