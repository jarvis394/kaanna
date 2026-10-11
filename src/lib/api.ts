import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type SourceKind = "system" | "input";
export interface Source {
  id: string;
  name: string;
  kind: SourceKind;
  isDefault: boolean;
}

export type ModelKind = "asr" | "mt" | "vad";
export interface Model {
  id: string;
  kind: ModelKind;
  name: string;
  description: string;
  size: number;
  installed: boolean;
}

export interface Settings {
  pauseMs: number;
  maxUtteranceSecs: number;
  livePreview: boolean;
  vadThreshold: number;
}

export interface Utterance {
  id: number;
  isFinal: boolean;
  source: string;
  translation: string;
  startedAt: number;
  audioMs: number;
  asrMs: number;
  mtMs: number;
}

export type Status =
  | { state: "idle" }
  | { state: "loading"; message: string }
  | { state: "listening" };

export interface DownloadProgress {
  id: string;
  received: number;
  total: number;
}

export interface Level {
  level: number;
  speaking: boolean;
}

export const api = {
  listSources: () => invoke<Source[]>("list_sources"),
  listModels: () => invoke<Model[]>("list_models"),
  defaultAsrModel: () => invoke<string>("default_asr_model"),
  downloadModel: (id: string) => invoke<void>("download_model", { id }),
  start: (sourceId: string, asrModel: string) => invoke<void>("start", { sourceId, asrModel }),
  stop: () => invoke<void>("stop"),
  getSettings: () => invoke<Settings>("get_settings"),
  setSettings: (settings: Settings) => invoke<void>("set_settings", { settings }),
};

type Events = {
  utterance: Utterance;
  "utterance-dropped": number;
  status: Status;
  "audio-level": Level;
  "download-progress": DownloadProgress;
  "pipeline-error": string;
};

export function on<K extends keyof Events>(event: K, cb: (payload: Events[K]) => void): Promise<UnlistenFn> {
  return listen<Events[K]>(event, (e) => cb(e.payload));
}

export function formatBytes(n: number): string {
  if (n >= 1e9) return `${(n / 1e9).toFixed(2)} GB`;
  if (n >= 1e6) return `${Math.round(n / 1e6)} MB`;
  return `${Math.max(1, Math.round(n / 1e3))} KB`;
}

/** localStorage that never throws (private mode, blocked storage…). */
export const prefs = {
  get<T>(key: string, fallback: T): T {
    try {
      const v = localStorage.getItem(`kaanna.${key}`);
      return v == null ? fallback : (JSON.parse(v) as T);
    } catch {
      return fallback;
    }
  },
  set(key: string, value: unknown) {
    try {
      localStorage.setItem(`kaanna.${key}`, JSON.stringify(value));
    } catch {
      /* ignore */
    }
  },
};
