import { getCurrentWindow } from "@tauri-apps/api/window";
import { Toggle } from "@base-ui/react/toggle";
import { Tooltip } from "@base-ui/react/tooltip";
import { useCallback, useEffect, useMemo, useState, type ReactElement } from "react";
import { CloseIcon, PinIcon, PlayIcon, SlidersIcon, StopIcon } from "./components/Icons";
import { LevelMeter } from "./components/LevelMeter";
import { Setup, type Downloads } from "./components/Models";
import { SettingsPanel, type ViewPrefs } from "./components/SettingsPanel";
import { SourceSelect } from "./components/SourceSelect";
import { Transcript } from "./components/Transcript";
import { api, on, prefs, type Model, type Settings, type Source, type Status, type Utterance } from "./lib/api";
import "./App.css";

const MAX_LINES = 60;
const MT_MODEL = "mt-opus-fi-en";
const VAD_MODEL = "vad-silero";

function Tip({ label, children }: { label: string; children: ReactElement }) {
  return (
    <Tooltip.Root>
      <Tooltip.Trigger render={children} />
      <Tooltip.Portal>
        <Tooltip.Positioner sideOffset={8}>
          <Tooltip.Popup className="tooltip">{label}</Tooltip.Popup>
        </Tooltip.Positioner>
      </Tooltip.Portal>
    </Tooltip.Root>
  );
}

export default function App() {
  const [sources, setSources] = useState<Source[]>([]);
  const [sourceId, setSourceId] = useState<string | null>(() => prefs.get("source", null));
  const [models, setModels] = useState<Model[]>([]);
  const [asrModel, setAsrModel] = useState<string | null>(() => prefs.get("asrModel", null));
  const [settings, setSettings] = useState<Settings | null>(null);
  const [view, setView] = useState<ViewPrefs>(() => prefs.get("view", { showOriginal: true, fontSize: 20 }));
  const [status, setStatus] = useState<Status>({ state: "idle" });
  const [lines, setLines] = useState<Utterance[]>([]);
  const [downloads, setDownloads] = useState<Downloads>({});
  const [error, setError] = useState<string | null>(null);
  const [panel, setPanel] = useState<"transcript" | "settings">("transcript");
  const [pinned, setPinned] = useState(true);
  const [hovered, setHovered] = useState(true);

  const refreshSources = useCallback(async () => {
    const list = await api.listSources();
    setSources(list);
    setSourceId((cur) => (cur && list.some((s) => s.id === cur) ? cur : (list[0]?.id ?? null)));
  }, []);
  const refreshModels = useCallback(() => api.listModels().then(setModels), []);

  // Initial load + backend events.
  useEffect(() => {
    refreshSources();
    refreshModels();
    api.defaultAsrModel().then((d) => setAsrModel((cur) => cur ?? d));
    api.getSettings().then((defaults) => {
      const s = { ...defaults, ...prefs.get<Partial<Settings>>("settings", {}) };
      setSettings(s);
      api.setSettings(s);
    });

    const subs = [
      on("status", setStatus),
      on("utterance", (u) =>
        setLines((prev) => {
          const i = prev.findIndex((p) => p.id === u.id);
          // A late draft must never overwrite the final text.
          if (i >= 0 && prev[i].isFinal && !u.isFinal) return prev;
          const next = i >= 0 ? prev.map((p, j) => (j === i ? u : p)) : [...prev, u].sort((a, b) => a.id - b.id);
          return next.slice(-MAX_LINES);
        }),
      ),
      on("utterance-dropped", (id) => setLines((prev) => prev.filter((p) => p.id !== id || p.isFinal))),
      on("download-progress", (p) => setDownloads((d) => ({ ...d, [p.id]: p }))),
      on("pipeline-error", setError),
    ];
    return () => {
      subs.forEach((s) => s.then((f) => f()));
    };
  }, [refreshSources, refreshModels]);

  const required = useMemo(
    () => [asrModel, MT_MODEL, VAD_MODEL].map((id) => models.find((m) => m.id === id)).filter((m): m is Model => !!m),
    [models, asrModel],
  );
  const ready = required.length === 3 && required.every((m) => m.installed);
  const listening = status.state === "listening";
  const loading = status.state === "loading";

  const start = useCallback(
    async (src = sourceId, model = asrModel) => {
      if (!src || !model) return;
      setError(null);
      try {
        await api.start(src, model);
      } catch (e) {
        setError(String(e));
      }
    },
    [sourceId, asrModel],
  );

  const download = useCallback(
    async (id: string) => {
      setError(null);
      try {
        await api.downloadModel(id);
      } catch (e) {
        setError(`Download failed: ${e}`);
      } finally {
        setDownloads((d) => ({ ...d, [id]: undefined }));
        await refreshModels();
      }
    },
    [refreshModels],
  );

  const downloadAll = async () => {
    // Small ones first so progress feels alive; one at a time keeps bandwidth focused.
    const missing = [...required].filter((m) => !m.installed).sort((a, b) => a.size - b.size);
    for (const m of missing) await download(m.id);
  };

  const changeSource = (id: string) => {
    setSourceId(id);
    prefs.set("source", id);
    if (listening) start(id, asrModel);
  };

  const changeAsrModel = (id: string) => {
    setAsrModel(id);
    prefs.set("asrModel", id);
    if (listening && models.find((m) => m.id === id)?.installed) start(sourceId, id);
  };

  const changeSettings = (s: Settings) => {
    setSettings(s);
    prefs.set("settings", s);
    api.setSettings(s);
  };

  const changeView = (v: ViewPrefs) => {
    setView(v);
    prefs.set("view", v);
  };

  const togglePin = async (v: boolean) => {
    setPinned(v);
    await getCurrentWindow().setAlwaysOnTop(v);
  };

  // While translating, fade the chrome away so the window reads like subtitles.
  const quiet = listening && !hovered && panel === "transcript" && lines.length > 0;

  return (
    <Tooltip.Provider delay={500}>
      <div
        className="app"
        data-quiet={quiet}
        onMouseEnter={() => setHovered(true)}
        onMouseLeave={() => setHovered(false)}
      >
        <header className="toolbar" data-tauri-drag-region>
          <div className="toolbar-left" data-tauri-drag-region>
            <span className="status-dot" data-state={status.state} />
            <SourceSelect sources={sources} value={sourceId} onChange={changeSource} onOpen={refreshSources} />
            <LevelMeter active={listening} />
          </div>
          <div className="toolbar-right" data-tauri-drag-region>
            {loading && <span className="toolbar-note">{status.message}</span>}
            <button
              className="btn btn-start"
              data-listening={listening}
              disabled={!ready || loading || !sourceId}
              onClick={() => (listening ? api.stop() : start())}
            >
              {listening ? <StopIcon width={12} height={12} /> : <PlayIcon width={12} height={12} />}
              {listening ? "Stop" : loading ? "Starting" : "Start"}
            </button>
            <Tip label="Keep on top">
              <Toggle className="icon-btn" pressed={pinned} onPressedChange={togglePin} aria-label="Keep on top">
                <PinIcon />
              </Toggle>
            </Tip>
            <Tip label="Settings">
              <Toggle
                className="icon-btn"
                pressed={panel === "settings"}
                onPressedChange={(p) => setPanel(p ? "settings" : "transcript")}
                aria-label="Settings"
              >
                <SlidersIcon />
              </Toggle>
            </Tip>
            <Tip label="Quit">
              <button className="icon-btn" onClick={() => getCurrentWindow().close()} aria-label="Quit">
                <CloseIcon />
              </button>
            </Tip>
          </div>
        </header>

        <main className="content">
          {panel === "settings" && settings && asrModel ? (
            <SettingsPanel
              settings={settings}
              onSettings={changeSettings}
              view={view}
              onView={changeView}
              models={models}
              asrModel={asrModel}
              onAsrModel={changeAsrModel}
              downloads={downloads}
              onDownload={download}
              onClear={() => setLines([])}
            />
          ) : !ready && models.length > 0 ? (
            <Setup required={required} downloads={downloads} error={error} onDownloadAll={downloadAll} />
          ) : (
            <Transcript lines={lines} showOriginal={view.showOriginal} fontSize={view.fontSize} listening={listening} />
          )}
        </main>

        {error && ready && (
          <div className="toast" role="alert">
            <span>{error}</span>
            <button className="icon-btn" onClick={() => setError(null)} aria-label="Dismiss">
              <CloseIcon width={14} height={14} />
            </button>
          </div>
        )}
      </div>
    </Tooltip.Provider>
  );
}
