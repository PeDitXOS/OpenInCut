/**
 * Web MIDI API wrapper: device detection, message parsing, CC→action mapping.
 * ponytail: skips sysex, MIDI2.0, multi-device routing. Add when needed.
 */

export type MidiAction =
  | { kind: "togglePlay" }
  | { kind: "split" }
  | { kind: "delete" }
  | { kind: "undo" }
  | { kind: "redo" }
  | { kind: "tool"; tool: "select" | "blade" | "trim" | "position" | "hand" | "zoom" }
  | { kind: "seek" } // CC 0..127 → proportionally across visible range
  | { kind: "shuttle"; direction: -1 | 1 };

export interface MidiMapping {
  /** "cc" for control change, "note" for note on/off */
  type: "cc" | "note";
  /** CC number (0-127) for type=cc, note number (0-127) for type=note */
  channel: number; // MIDI channel 0-15 (-1 = any)
  number: number; // CC or note number
  action: MidiAction;
}

export interface MidiPreset {
  name: string;
  mappings: MidiMapping[];
}

const STORAGE_KEY = "opencut.midi.mappings";
const PRESET_KEY = "opencut.midi.presets";

/** Try to get MIDI access. Returns null if unavailable (non-HTTPS, no browser support). */
export async function requestMidiAccess(): Promise<MIDIAccess | null> {
  try {
    if (!navigator.requestMIDIAccess) return null;
    return await navigator.requestMIDIAccess();
  } catch {
    return null;
  }
}

/** List connected MIDI input devices. */
export function listInputDevices(access: MIDIAccess): MIDIInput[] {
  return [...access.inputs.values()];
}

/** Parse raw MIDI message bytes into a structured object. */
export function parseMidiMessage(data: Uint8Array): {
  status: number;
  command: number;
  channel: number;
  data1: number;
  data2: number;
} | null {
  if (data.length < 1) return null;
  const status = data[0];
  const command = status & 0xf0;
  const channel = status & 0x0f;
  // Ignore system messages (status < 0x80) and sysex
  if (command < 0x80) return null;
  return {
    status,
    command,
    channel,
    data1: data.length > 1 ? data[1] : 0,
    data2: data.length > 2 ? data[2] : 0,
  };
}

/** Match a parsed MIDI message against a mapping. */
export function matchMapping(
  msg: { command: number; channel: number; data1: number; data2: number },
  mapping: MidiMapping,
): boolean {
  if (mapping.channel !== -1 && mapping.channel !== msg.channel) return false;
  if (mapping.number !== msg.data1) return false;

  if (mapping.type === "cc") {
    return msg.command === 0xb0; // Control Change
  }
  if (mapping.type === "note") {
    // Match note-on (velocity > 0) — note-off is ignored
    return msg.command === 0x90 && msg.data2 > 0;
  }
  return false;
}

/** Load saved mappings from localStorage. */
export function loadMappings(): MidiMapping[] {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    return raw ? JSON.parse(raw) : [];
  } catch {
    return [];
  }
}

/** Save mappings to localStorage. */
export function saveMappings(mappings: MidiMapping[]): void {
  localStorage.setItem(STORAGE_KEY, JSON.stringify(mappings));
}

/** Load preset list from localStorage. */
export function loadPresets(): MidiPreset[] {
  try {
    const raw = localStorage.getItem(PRESET_KEY);
    return raw ? JSON.parse(raw) : [];
  } catch {
    return [];
  }
}

/** Save preset list to localStorage. */
export function savePresets(presets: MidiPreset[]): void {
  localStorage.setItem(PRESET_KEY, JSON.stringify(presets));
}

/** Default mapping: CC7 → volume-like seek, Note 36 (C2) → play/pause. */
export const DEFAULT_MAPPINGS: MidiMapping[] = [
  { type: "note", channel: -1, number: 36, action: { kind: "togglePlay" } },
  { type: "note", channel: -1, number: 42, action: { kind: "split" } },
  { type: "note", channel: -1, number: 43, action: { kind: "delete" } },
  { type: "note", channel: -1, number: 44, action: { kind: "undo" } },
  { type: "note", channel: -1, number: 45, action: { kind: "redo" } },
  { type: "cc", channel: -1, number: 7, action: { kind: "seek" } },
  { type: "cc", channel: -1, number: 1, action: { kind: "shuttle", direction: 1 } },
];
