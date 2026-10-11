// Browser-only demo backend, so the UI can be developed with plain `bun run dev`
// (no Rust build, no models). Never loaded inside the Tauri app.
import { emit } from "@tauri-apps/api/event";
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import type { Model, Settings, Source, Utterance } from "./api";

const SCRIPT: [string, string][] = [
  ["Hyvää iltaa ja tervetuloa uutisten pariin.", "Good evening and welcome to the news."],
  ["Helsingissä sataa tänään lunta koko päivän.", "In Helsinki, it will snow all day today."],
  ["Hallitus esitteli eilen uuden budjettinsa.", "The government presented its new budget yesterday."],
  ["Liikenne ruuhkautui pahasti aamulla Kehä ykkösellä.", "Traffic was badly congested on Ring Road I this morning."],
];

export function installDemoBackend() {
  mockWindows("main");
  const sources: Source[] = [
    { id: "system", name: "System audio", kind: "system", isDefault: false },
    { id: "input:mic", name: "MacBook Air Microphone", kind: "input", isDefault: true },
    { id: "input:bh", name: "BlackHole 2ch", kind: "input", isDefault: false },
  ];
  const models: Model[] = [
    { id: "asr-fi-large-v3-q5", kind: "asr", name: "Whisper Large v3 Finnish · Q5", description: "Demo", size: 1_081_140_203, installed: true },
    { id: "asr-fi-large-v3", kind: "asr", name: "Whisper Large v3 Finnish · F16", description: "Demo", size: 3_095_033_483, installed: false },
    { id: "mt-opus-fi-en", kind: "mt", name: "OPUS-MT Finnish → English (big)", description: "Demo", size: 473_586_856, installed: true },
    { id: "vad-silero", kind: "vad", name: "Silero VAD v6", description: "Demo", size: 885_098, installed: true },
  ];
  let settings: Settings = { pauseMs: 600, maxUtteranceSecs: 12, livePreview: true, vadThreshold: 0.5 };
  let timers: number[] = [];
  const downloads = new Map<string, () => void>();

  // Fake download: ~6 s of progress events, resolving true when done or false when cancelled.
  const download = (id: string) =>
    new Promise<boolean>((resolve) => {
      const model = models.find((m) => m.id === id)!;
      let received = 0;
      const timer = window.setInterval(() => {
        received = Math.min(model.size, received + model.size / 30);
        emit("download-progress", { id, received, total: model.size });
        if (received >= model.size) finish(true);
      }, 200);
      const finish = (done: boolean) => {
        clearInterval(timer);
        downloads.delete(id);
        model.installed ||= done;
        resolve(done);
      };
      downloads.set(id, () => finish(false));
    });

  const play = () => {
    let t = 300;
    SCRIPT.forEach(([fi, en], i) => {
      const id = Date.now() + i;
      const base: Utterance = { id, isFinal: false, source: "", translation: "", startedAt: id, audioMs: 2400, asrMs: 610, mtMs: 140 };
      const half = fi.split(" ").slice(0, 3).join(" ");
      const halfEn = en.split(" ").slice(0, 3).join(" ");
      timers.push(window.setTimeout(() => emit("utterance", { ...base, source: half, translation: halfEn }), t));
      timers.push(window.setTimeout(() => emit("utterance", { ...base, isFinal: true, source: fi, translation: en }), t + 1400));
      t += 2600;
    });
    let phase = 0;
    timers.push(
      window.setInterval(() => {
        phase += 0.4;
        emit("audio-level", { level: 0.45 + 0.35 * Math.abs(Math.sin(phase)), speaking: true });
      }, 70),
    );
  };

  mockIPC(
    (cmd, args) => {
      switch (cmd) {
        case "list_sources":
          return sources;
        case "list_models":
          return models.map((m) => ({ ...m }));
        case "default_asr_model":
          return "asr-fi-large-v3-q5";
        case "get_settings":
          return settings;
        case "set_toggle_shortcut":
          return;
        case "set_settings":
          settings = (args as { settings: Settings }).settings;
          return;
        case "start":
          timers.forEach(clearTimeout);
          timers = [];
          emit("status", { state: "listening" });
          play();
          return;
        case "download_model":
          return download((args as { id: string }).id);
        case "delete_model": {
          const model = models.find((m) => m.id === (args as { id: string }).id);
          if (model) model.installed = false;
          return;
        }
        case "cancel_download":
          downloads.get((args as { id: string }).id)?.();
          return;
        case "stop":
          timers.forEach(clearTimeout);
          timers = [];
          emit("status", { state: "idle" });
          return;
        default:
          return;
      }
    },
    { shouldMockEvents: true },
  );
}
