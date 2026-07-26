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
  number: number;
  /** MIDI channel 0-15 (-1 = any) */
  channel: number;
  action: MidiAction;
}

export interface MidiDevice {
  id: string;
  name: string;
  manufacturer?: string;
  enabled: boolean;
}

export interface MidiPreset {
  name: string;
  mappings: MidiMapping[];
  devices: MidiDevice[];
  backlightCc?: number;
  backlightChannel?: number;
  backlightValue?: number;
}

const STORAGE_KEY = "opencut.midi.mappings";
const PRESET_KEY = "opencut.midi.presets";
const DEVICE_KEY = "opencut.midi.devices";

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

/** List connected MIDI output devices. */
export function listOutputDevices(access: MIDIAccess): MIDIOutput[] {
  return [...access.outputs.values()];
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

/** Load saved device configs from localStorage. */
export function loadDevices(): MidiDevice[] {
  try {
    const raw = localStorage.getItem(DEVICE_KEY);
    return raw ? JSON.parse(raw) : [];
  } catch {
    return [];
  }
}

/** Save device configs to localStorage. */
export function saveDevices(devices: MidiDevice[]): void {
  localStorage.setItem(DEVICE_KEY, JSON.stringify(devices));
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

/** Save a preset with current mappings and device config. */
export function savePreset(name: string, mappings: MidiMapping[], devices: MidiDevice[], backlight?: { cc: number; channel: number; value: number }): void {
  const presets = loadPresets();
  const next = [...presets.filter((p) => p.name !== name), { name, mappings, devices, ...backlight }];
  savePresets(next);
}

/** Load a preset by name, returning mappings and device config. */
export function loadPreset(name: string): MidiPreset | undefined {
  const presets = loadPresets();
  return presets.find((p) => p.name === name);
}

/** Delete a preset by name. */
export function deletePreset(name: string): void {
  const presets = loadPresets().filter((p) => p.name !== name);
  savePresets(presets);
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

/** Dispatch a MIDI action to the store. */
export function dispatchMidi(
  action: MidiAction,
  value: number,
  getState: () => {
    midiMappings: MidiMapping[];
    midiDeviceId: string | null;
    playheadUs: number;
    viewStartUs: number;
    pxPerSec: number;
    togglePlay(): void;
    splitAtPlayhead(): Promise<void>;
    deleteSelection(ripple: boolean): Promise<void>;
    undo(): Promise<void>;
    redo(): Promise<void>;
    seek(us: number): void;
    shuttle(direction: -1 | 0 | 1): void;
    setTool(tool: string): void;
  },
): void {
  const s = getState();
  switch (action.kind) {
    case "togglePlay": s.togglePlay(); break;
    case "split": void s.splitAtPlayhead(); break;
    case "delete": void s.deleteSelection(false); break;
    case "undo": void s.undo(); break;
    case "redo": void s.redo(); break;
    case "seek": {
      const viewLen = Math.max(1, s.pxPerSec * 10);
      const target = s.viewStartUs + (value / 127) * viewLen * 1_000_000 / s.pxPerSec;
      s.seek(target);
      break;
    }
    case "shuttle": {
      const dir = value > 64 ? action.direction : value < 63 ? -action.direction : 0;
      s.shuttle(dir as -1 | 0 | 1);
      break;
    }
    case "tool": s.setTool(action.tool); break;
  }
}

/** Send Note On for LED feedback to a MIDI output. */
export async function sendNoteOn(output: MIDIOutput, note: number, velocity: number, channel = 0): Promise<void> {
  try {
    output.send([0x90 | (channel & 0x0f), note & 0x7f, velocity & 0x7f]);
  } catch {}
}

/** Send Control Change for LED/backlight feedback to a MIDI output. */
export async function sendCC(output: MIDIOutput, cc: number, value: number, channel = 0): Promise<void> {
  try {
    output.send([0xb0 | (channel & 0x0f), cc & 0x7f, value & 0x7f]);
  } catch {}
}

/** Send backlight/brightness CC to all enabled output devices. */
export async function sendBacklight(
  access: MIDIAccess,
  devices: MidiDevice[],
  cc: number,
  value: number,
  channel = 0,
): Promise<void> {
  for (const d of devices) {
    if (!d.enabled) continue;
    const output = access.outputs.get(d.id);
    if (output) {
      await sendCC(output, cc, value, channel);
    }
  }
}

/** Send LED feedback (Note On) when a MIDI action is triggered. */
export async function sendLedFeedback(
  access: MIDIAccess,
  devices: MidiDevice[],
  mapping: MidiMapping,
  value: number,
): Promise<void> {
  // For note mappings, send note-on with velocity based on action
  if (mapping.type === "note") {
    for (const d of devices) {
      if (!d.enabled) continue;
      const output = access.outputs.get(d.id);
      if (output) {
        await sendNoteOn(output, mapping.number, value > 0 ? 127 : 0, mapping.channel);
      }
    }
  }
  // For CC mappings, could send CC feedback with same value
  if (mapping.type === "cc") {
    for (const d of devices) {
      if (!d.enabled) continue;
      const output = access.outputs.get(d.id);
      if (output) {
        await sendCC(output, mapping.number, value, mapping.channel);
      }
    }
  }
}

/** Initialize MIDI controller listener for multiple devices. Returns cleanup function. */
export function initMidi(
  getState: () => {
    midiDeviceIds: string[];
    midiMappings: MidiMapping[];
    midiDevices: MidiDevice[];
    playheadUs: number;
    viewStartUs: number;
    pxPerSec: number;
    togglePlay(): void;
    splitAtPlayhead(): Promise<void>;
    deleteSelection(ripple: boolean): Promise<void>;
    undo(): Promise<void>;
    redo(): Promise<void>;
    seek(us: number): void;
    shuttle(direction: -1 | 0 | 1): void;
    setTool(tool: string): void;
  },
): () => void {
  let cleanup: (() => void) | undefined;

  void requestMidiAccess().then((access) => {
    if (!access) return;
    const { midiDeviceIds, midiMappings, midiDevices } = getState();
    if (!midiDeviceIds.length || !midiDevices.length) return;

    const cleanupFns: (() => void)[] = [];

    for (const deviceId of midiDeviceIds) {
      const deviceConfig = midiDevices.find(d => d.id === deviceId && d.enabled);
      if (!deviceConfig) continue;

      const input = access.inputs.get(deviceId);
      if (!input) continue;

      input.onmidimessage = (e: MIDIMessageEvent) => {
        if (!e.data) return;
        const msg = parseMidiMessage(e.data);
        if (!msg) return;

        for (const m of midiMappings) {
          if (matchMapping(msg, m)) {
            dispatchMidi(m.action, msg.data2, getState as () => any);
            // Send LED feedback to all enabled output devices
            void sendLedFeedback(access, midiDevices, m, msg.data2);
            break;
          }
        }
      };
      cleanupFns.push(() => { input.onmidimessage = null; });
    }

    cleanup = () => { cleanupFns.forEach(fn => fn()); };
  });

  return () => { cleanup?.(); };
}