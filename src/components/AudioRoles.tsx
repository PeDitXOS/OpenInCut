import { useState } from "react";

/** Audio Roles: Video, Dialogue, Music, SFX — each with Mute/Solo/Lock + volume fader. */

interface RoleState {
  muted: boolean;
  solo: boolean;
  locked: boolean;
  volume: number; // dB, -60..12
}

const ROLES = [
  { key: "video", label: "Video", color: "var(--color-clip-video)" },
  { key: "dialogue", label: "Dialogue", color: "var(--color-clip-audio)" },
  { key: "music", label: "Music", color: "var(--color-clip-text)" },
  { key: "sfx", label: "SFX", color: "var(--color-clip-effects)" },
] as const;

const defaultRoles: Record<string, RoleState> = {
  video: { muted: false, solo: false, locked: false, volume: 0 },
  dialogue: { muted: false, solo: false, locked: false, volume: 0 },
  music: { muted: false, solo: false, locked: false, volume: -6 },
  sfx: { muted: false, solo: false, locked: false, volume: -3 },
};

export function AudioRoles() {
  const [roles, setRoles] = useState<Record<string, RoleState>>({
    ...defaultRoles,
  });

  const toggle = (key: string, field: "muted" | "solo" | "locked") =>
    setRoles((r) => ({
      ...r,
      [key]: { ...r[key], [field]: !r[key][field] },
    }));

  const setVolume = (key: string, v: number) =>
    setRoles((r) => ({
      ...r,
      [key]: { ...r[key], volume: v },
    }));

  const hasSolo = Object.values(roles).some((r) => r.solo);

  return (
    <section className="border-b border-line-soft px-3 py-3">
      <h3 className="panel-eyebrow mb-2">Audio Roles</h3>
      <div className="space-y-2">
        {ROLES.map(({ key, label, color }) => {
          const role = roles[key];
          const dimmed = hasSolo && !role.solo;
          return (
            <div
              key={key}
              className={`flex items-center gap-2 rounded-md border border-line bg-bg2/50 px-2 py-1.5 text-[11px] ${dimmed ? "opacity-40" : ""}`}
            >
              {/* Color dot + label */}
              <div className="flex w-20 shrink-0 items-center gap-1.5">
                <div
                  className="h-2 w-2 shrink-0 rounded-full"
                  style={{ background: color }}
                />
                <span className="truncate text-ink">{label}</span>
              </div>

              {/* Mute / Solo / Lock buttons */}
              {(["muted", "solo", "locked"] as const).map((field) => (
                <button
                  key={field}
                  className={`focus-ring shrink-0 rounded px-1.5 py-0.5 text-[10px] font-medium ${
                    role[field]
                      ? field === "muted"
                        ? "bg-danger/20 text-danger"
                        : field === "solo"
                          ? "bg-accent/20 text-accent"
                          : "bg-warning/20 text-warning"
                      : "bg-bg3 text-ink-faint hover:text-ink-dim"
                  }`}
                  onClick={() => toggle(key, field)}
                  title={field === "muted" ? "Mute" : field === "solo" ? "Solo" : "Lock"}
                >
                  {field === "muted" ? "M" : field === "solo" ? "S" : "🔒"}
                </button>
              ))}

              {/* Volume fader */}
              <input
                type="range"
                className="h-1 min-w-0 flex-1 cursor-pointer appearance-none rounded-full bg-bg3 accent-(--color-accent)"
                min={-60}
                max={12}
                step={0.5}
                value={role.volume}
                onChange={(e) => setVolume(key, Number(e.target.value))}
              />
              <span className="w-10 shrink-0 text-right font-[var(--font-mono)] text-[10px] text-ink-faint">
                {role.volume > -60 ? `${role.volume.toFixed(0)} dB` : "−∞"}
              </span>
            </div>
          );
        })}
      </div>

      {/* Reset */}
      <button
        className="focus-ring mt-2 w-full rounded-md border border-line bg-bg2 px-2 py-1.5 text-[11px] text-ink-dim hover:text-ink"
        onClick={() => setRoles({ ...defaultRoles })}
      >
        ↻ Reset all roles
      </button>
    </section>
  );
}
