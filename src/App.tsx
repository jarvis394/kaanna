import { getCurrentWindow } from "@tauri-apps/api/window";
import { Toggle } from "@base-ui/react/toggle";
import { Tooltip } from "@base-ui/react/tooltip";
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactElement } from "react";
import { CloseIcon, PinIcon, PlayIcon, SlidersIcon, StopIcon } from "./components/Icons";
import { LevelMeter } from "./components/LevelMeter";
import { Setup, type Downloads } from "./components/Models";
import { SettingsPanel, type ViewPrefs } from "./components/SettingsPanel";
import { DEFAULT_TOGGLE_SHORTCUT } from "./components/ShortcutRecorder";
import { SourceSelect } from "./components/SourceSelect";
import { Spinner } from "./components/Spinner";
import { Transcript } from "./components/Transcript";
import { usePresence } from "./lib/presence";
import { api, isMac, on, platform, prefs, type Device, type Gpu, type Model, type Settings, type Source, type Status, type Utterance } from "./lib/api";
import "./App.css";

const MAX_LINES = 60;
const MT_MODEL = "mt-opus-fi-en";
const VAD_MODEL = "vad-silero";

const UNDO_MS = 5000;

function Tip({ label, shortcut, children }: { label: string; shortcut?: string; children: ReactElement }) {
  return (
    <Tooltip.Root>
      <Tooltip.Trigger render={children} />
      <Tooltip.Portal>
        <Tooltip.Positioner sideOffset={8}>
          <Tooltip.Popup className="tooltip">
            {label}
            {shortcut && <kbd className="tooltip-kbd">{shortcut}</kbd>}
          </Tooltip.Popup>
        </Tooltip.Positioner>
      </Tooltip.Portal>
    </Tooltip.Root>
  );
}

/** Keys typed into controls belong to them, not to the app shortcuts. */
function isControl(t: EventTarget | null) {
  return t instanceof Element && !!t.closest("button, input, textarea, select, [role], [contenteditable]");
}

export default function App() {
  const [sources, setSources] = useState<Source[]>([]);
  const [sourceId, setSourceId] = useState<string | null>(() => prefs.get("source", null));
  const [models, setModels] = useState<Model[]>([]);
  const [asrModel, setAsrModel] = useState<string | null>(() => prefs.get("asrModel", null));
  const [gpus, setGpus] = useState<Gpu[]>([]);
  const [device, setDevice] = useState<Device>(() => prefs.get("device", "auto"));
  const [settings, setSettings] = useState<Settings | null>(null);
  const [view, setView] = useState<ViewPrefs>(() => prefs.get("view", { showOriginal: true, fontSize: 20 }));
  const [status, setStatus] = useState<Status>({ state: "idle" });
  const [lines, setLines] = useState<Utterance[]>([]);
  const [downloads, setDownloads] = useState<Downloads>({});
  const [error, setError] = useState<string | null>(null);
  const [panel, setPanel] = useState<"transcript" | "settings">("transcript");
  const [pinned, setPinned] = useState(true);
  const [hovered, setHovered] = useState(true);
  const [toggleShortcut, setToggleShortcut] = useState(() => prefs.get("toggleShortcut", DEFAULT_TOGGLE_SHORTCUT));
  const [cleared, setCleared] = useState<Utterance[] | null>(null);
  const undoTimer = useRef<number | undefined>(undefined);

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
    api.listGpus().then(setGpus).catch(() => setGpus([]));
    api.defaultAsrModel().then((d) => setAsrModel((cur) => cur ?? d));
    api.getSettings().then((defaults) => {
      const s = { ...defaults, ...prefs.get<Partial<Settings>>("settings", {}) };
      setSettings(s);
      api.setSettings(s);
    });

    api.setToggleShortcut(prefs.get("toggleShortcut", DEFAULT_TOGGLE_SHORTCUT)).catch((e) => setError(`Show/hide shortcut: ${e}`));

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

  // The window starts hidden. Reveal it once the model list is in, so the first frame is already
  // the right screen (setup or transcript), and after a paint so it's never blank.
  const hasModels = models.length > 0;
  useEffect(() => {
    if (!hasModels) return;
    requestAnimationFrame(() => requestAnimationFrame(() => api.revealWindow()));
  }, [hasModels]);

  const required = useMemo(
    () => [asrModel, MT_MODEL, VAD_MODEL].map((id) => models.find((m) => m.id === id)).filter((m): m is Model => !!m),
    [models, asrModel],
  );
  const ready = required.length === 3 && required.every((m) => m.installed);
  const listening = status.state === "listening";
  const loading = status.state === "loading";

  const start = useCallback(
    async (src = sourceId, model = asrModel, dev = device) => {
      if (!src || !model) return;
      setError(null);
      try {
        await api.start(src, model, dev);
      } catch (e) {
        setError(String(e));
      }
    },
    [sourceId, asrModel, device],
  );

  // Models with a download running, so a second click (or "Download all") never starts a duplicate,
  // whose immediate "already in progress" reply would clear the first one's progress.
  const inFlight = useRef(new Set<string>());
  const download = useCallback(
    async (id: string) => {
      if (inFlight.current.has(id)) return false;
      inFlight.current.add(id);
      setError(null);
      // Show progress (and the Cancel button) right away, before the first byte arrives.
      const size = models.find((m) => m.id === id)?.size ?? 0;
      setDownloads((d) => ({ ...d, [id]: d[id] ?? { id, received: 0, total: size } }));
      try {
        return await api.downloadModel(id);
      } catch (e) {
        setError(`Download failed: ${e}`);
        return false;
      } finally {
        inFlight.current.delete(id);
        setDownloads((d) => ({ ...d, [id]: undefined }));
        await refreshModels();
      }
    },
    [models, refreshModels],
  );

  const cancelDownload = (id: string) => {
    api.cancelDownload(id).catch((e) => setError(String(e)));
  };

  const deleteModel = async (id: string) => {
    setError(null);
    try {
      await api.deleteModel(id);
    } catch (e) {
      setError(`Couldn't delete the model: ${e}`);
    } finally {
      await refreshModels();
    }
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

  // The model reloads onto the new processor on the next start; restart now if translating.
  const changeDevice = (d: Device) => {
    setDevice(d);
    prefs.set("device", d);
    if (listening) start(sourceId, asrModel, d);
  };

  const changeSettings = (s: Settings) => {
    setSettings(s);
    prefs.set("settings", s);
    api.setSettings(s);
  };

  const changeToggleShortcut = (s: string) => {
    setToggleShortcut(s);
    prefs.set("toggleShortcut", s);
  };

  const changeView = (v: ViewPrefs) => {
    setView(v);
    prefs.set("view", v);
  };

  // Clearing is instant; the old lines stay recoverable for a few seconds instead of asking first.
  const clear = useCallback(() => {
    if (lines.length === 0) return;
    setCleared(lines);
    setLines([]);
    window.clearTimeout(undoTimer.current);
    undoTimer.current = window.setTimeout(() => setCleared(null), UNDO_MS);
  }, [lines]);

  const undoClear = useCallback(() => {
    if (!cleared) return;
    // Lines that arrived after clearing stay, after the restored ones.
    setLines((prev) => [...cleared, ...prev].slice(-MAX_LINES));
    setCleared(null);
    window.clearTimeout(undoTimer.current);
  }, [cleared]);

  const canStart = ready && !loading && !!sourceId;
  // While models load the button already reads Stop, and pressing it abandons the load.
  const busy = listening || loading;
  const toggleListening = useCallback(() => {
    if (busy) api.stop();
    else if (canStart) start();
  }, [busy, canStart, start]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const cmd = e.metaKey || e.ctrlKey;
      if (cmd && e.key === ",") {
        e.preventDefault();
        setPanel((p) => (p === "settings" ? "transcript" : "settings"));
      } else if (cmd && e.key.toLowerCase() === "k") {
        e.preventDefault();
        clear();
      } else if (cmd && !e.shiftKey && e.key.toLowerCase() === "z" && cleared) {
        e.preventDefault();
        undoClear();
      } else if (e.key === "Escape" && !e.defaultPrevented && !document.querySelector('[role="listbox"]')) {
        setPanel("transcript");
      } else if (e.key === " " && !cmd && !e.repeat && !isControl(e.target)) {
        e.preventDefault();
        toggleListening();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [clear, undoClear, cleared, toggleListening]);

  const togglePin = async (v: boolean) => {
    setPinned(v);
    await getCurrentWindow().setAlwaysOnTop(v);
  };

  // While translating, fade the chrome away so the window reads like subtitles.
  const quiet = listening && !hovered && panel === "transcript" && lines.length > 0;
  useEffect(() => {
    if (isMac) api.setWindowControlsVisible(!quiet);
  }, [quiet]);
  const screen = panel === "settings" && settings && asrModel ? "settings" : !ready && models.length > 0 ? "setup" : "transcript";
  const startLabel = busy ? "Stop" : "Start";
  // The label box animates to the active word's width, so the button resizes smoothly instead of jumping.
  const labelsRef = useRef<HTMLSpanElement>(null);
  const [labelWidth, setLabelWidth] = useState<number>();
  useLayoutEffect(() => {
    const active = labelsRef.current?.querySelector<HTMLElement>('[data-visible="true"]');
    if (!active) return;
    // Measure the text itself: WebKit reports the grid item's box wider than its content.
    const range = document.createRange();
    range.selectNodeContents(active);
    setLabelWidth(Math.ceil(range.getBoundingClientRect().width));
  }, [startLabel]);
  // Exit durations match the CSS below so leaving elements finish fading before they unmount.
  const [note, noteExiting] = usePresence(loading ? (status.message ?? null) : null, 120);
  const [clearedToast, clearedExiting] = usePresence(cleared, 150);
  const [errorToast, errorExiting] = usePresence(error && ready ? error : null, 150);
  const startBlocked = !ready ? "Download the models first" : !sourceId ? "Choose an audio source first" : null;

  return (
    <Tooltip.Provider delay={500}>
      <div
        className="app"
        data-platform={platform}
        data-quiet={quiet}
        data-view={screen}
        onMouseEnter={() => setHovered(true)}
        onMouseLeave={() => setHovered(false)}
      >
        <header className="toolbar" data-tauri-drag-region>
          <div className="toolbar-left" data-tauri-drag-region>
            <SourceSelect sources={sources} value={sourceId} onChange={changeSource} onOpen={refreshSources} state={status.state} />
            <LevelMeter active={listening} />
          </div>
          <div className="toolbar-right" data-tauri-drag-region>
            {note && (
              <span className="toolbar-note" data-exiting={noteExiting || undefined}>
                {note}
              </span>
            )}
            <Tip
              label={loading ? "Cancel loading" : listening ? "Stop translating" : (startBlocked ?? "Start translating")}
              shortcut={!busy && startBlocked ? undefined : "Space"}
            >
              {/* Disabled buttons swallow pointer events, so the wrapper carries the tooltip. */}
              <span className="tip-wrap">
                <button
                  className="btn btn-start"
                  data-listening={busy}
                  disabled={!busy && !canStart}
                  onClick={toggleListening}
                >
                  <span className="swap-stack">
                    <span className="swap" data-visible={!busy}>
                      <PlayIcon width={16} height={16} />
                    </span>
                    <span className="swap" data-visible={listening}>
                      <StopIcon width={16} height={16} />
                    </span>
                    <span className="swap" data-visible={loading}>
                      <Spinner size={14} />
                    </span>
                  </span>
                  {/* All labels share one grid cell so they can cross-fade in place. */}
                  <span className="swap-stack start-labels" ref={labelsRef} style={{ width: labelWidth }}>
                    {(["Start", "Stop"] as const).map((l) => (
                      <span key={l} className="swap swap-label" data-visible={l === startLabel} aria-hidden={l !== startLabel}>
                        {l}
                      </span>
                    ))}
                  </span>
                </button>
              </span>
            </Tip>
            <Tip label="Keep on top">
              <Toggle className="icon-btn" pressed={pinned} onPressedChange={togglePin} aria-label="Keep on top">
                <PinIcon />
              </Toggle>
            </Tip>
            <Tip label="Settings" shortcut="⌘,">
              <Toggle
                className="icon-btn"
                pressed={panel === "settings"}
                onPressedChange={(p) => setPanel(p ? "settings" : "transcript")}
                aria-label="Settings"
              >
                <SlidersIcon />
              </Toggle>
            </Tip>
            {/* macOS has the native close button; elsewhere close sits top-right, as on Windows.
                Closing hides the window; the tray icon brings it back. */}
            {!isMac && (
              <Tip label="Close">
                <button className="icon-btn" onClick={() => getCurrentWindow().close()} aria-label="Close window">
                  <CloseIcon />
                </button>
              </Tip>
            )}
          </div>
        </header>

        <main className="content">
          {screen === "settings" && settings && asrModel ? (
            <SettingsPanel
              settings={settings}
              onSettings={changeSettings}
              view={view}
              onView={changeView}
              models={models}
              asrModel={asrModel}
              onAsrModel={changeAsrModel}
              required={required}
              gpus={gpus}
              device={device}
              onDevice={changeDevice}
              downloads={downloads}
              onDownload={download}
              onCancelDownload={cancelDownload}
              onDeleteModel={deleteModel}
              onClear={clear}
              toggleShortcut={toggleShortcut}
              onToggleShortcut={changeToggleShortcut}
            />
          ) : screen === "setup" ? (
            <Setup required={required} downloads={downloads} error={error} onDownload={download} onCancel={cancelDownload} />
          ) : (
            <Transcript lines={lines} showOriginal={view.showOriginal} fontSize={view.fontSize} listening={listening} />
          )}
        </main>

        <div className="toasts">
          {clearedToast && (
            <div className="toast" role="status" data-exiting={clearedExiting || undefined}>
              <span>Transcript cleared</span>
              <Tip label="Undo" shortcut="⌘Z">
                <button className="btn btn-ghost btn-sm" onClick={undoClear}>
                  Undo
                </button>
              </Tip>
            </div>
          )}
          {errorToast && (
            <div className="toast toast-error" role="alert" data-exiting={errorExiting || undefined}>
              <span>{errorToast}</span>
              <button className="icon-btn" onClick={() => setError(null)} aria-label="Dismiss">
                <CloseIcon width={14} height={14} />
              </button>
            </div>
          )}
        </div>
      </div>
    </Tooltip.Provider>
  );
}
