import { Select } from "@base-ui/react/select";
import { Slider } from "@base-ui/react/slider";
import { Switch } from "@base-ui/react/switch";
import type { ReactNode } from "react";
import { formatBytes, thisDevice, type Device, type Gpu, type Model, type Settings } from "../lib/api";
import { CheckIcon, ChevronIcon } from "./Icons";
import { ModelRow, StoredModelRow, type Downloads } from "./Models";
import { ShortcutRecorder } from "./ShortcutRecorder";

export interface ViewPrefs {
  showOriginal: boolean;
  fontSize: number;
}

interface Props {
  settings: Settings;
  onSettings: (s: Settings) => void;
  view: ViewPrefs;
  onView: (v: ViewPrefs) => void;
  models: Model[];
  asrModel: string;
  onAsrModel: (id: string) => void;
  /** The selected speech model plus translation and voice detection: everything Start needs. */
  required: Model[];
  gpus: Gpu[];
  device: Device;
  onDevice: (d: Device) => void;
  downloads: Downloads;
  onDownload: (id: string) => void;
  onCancelDownload: (id: string) => void;
  onDeleteModel: (id: string) => void;
  onClear: () => void;
  toggleShortcut: string;
  onToggleShortcut: (accelerator: string) => void;
}

function Row({ label, hint, children }: { label: string; hint?: string; children: ReactNode }) {
  return (
    <div className="setting-row">
      <div className="setting-label">
        <span>{label}</span>
        {hint && <span className="setting-hint">{hint}</span>}
      </div>
      <div className="setting-control">{children}</div>
    </div>
  );
}

function Toggle({ checked, onChange, label }: { checked: boolean; onChange: (v: boolean) => void; label: string }) {
  return (
    <Switch.Root className="switch" checked={checked} onCheckedChange={onChange} aria-label={label}>
      <Switch.Thumb className="switch-thumb" />
    </Switch.Root>
  );
}

function RangeSlider(props: { value: number; min: number; max: number; step: number; onChange: (v: number) => void; format: (v: number) => string; label: string }) {
  return (
    <div className="range">
      <Slider.Root
        className="slider"
        value={props.value}
        min={props.min}
        max={props.max}
        step={props.step}
        onValueChange={(v) => props.onChange(v as number)}
      >
        <Slider.Control className="slider-control">
          <Slider.Track className="slider-track">
            <Slider.Indicator className="slider-indicator" />
            <Slider.Thumb className="slider-thumb" aria-label={props.label} />
          </Slider.Track>
        </Slider.Control>
      </Slider.Root>
      <span className="range-value">{props.format(props.value)}</span>
    </div>
  );
}

/** Mirrors the backend's `resolve_device`: a dedicated GPU first, then an integrated one. */
function autoGpu(gpus: Gpu[]) {
  return gpus.find((g) => !g.integrated) ?? gpus[0];
}

function gpuHint(g: Gpu) {
  // Video memory is sold in binary gigabytes: an "8 GB" card reports 8.59e9 bytes.
  return g.integrated ? "Integrated" : `Dedicated · ${Math.round(g.memory / 2 ** 30)} GB`;
}

/** Names the GPU only for Automatic; otherwise the menu already shows it. */
function deviceHint(gpus: Gpu[], device: Device) {
  const explicit = device === "cpu" || gpus.some((g) => `gpu:${g.index}` === device);
  return explicit ? "Runs speech recognition" : `Using ${autoGpu(gpus).name}`;
}

function ProcessorSelect({ gpus, device, onDevice }: { gpus: Gpu[]; device: Device; onDevice: (d: Device) => void }) {
  // A GPU that's gone (e.g. an unplugged eGPU) falls back to automatic, as in the backend.
  const value = device === "cpu" || gpus.some((g) => `gpu:${g.index}` === device) ? device : "auto";
  const options = [
    { value: "auto", label: "Automatic", hint: "Prefers dedicated" },
    ...gpus.map((g) => ({ value: `gpu:${g.index}`, label: g.name, hint: gpuHint(g) })),
    { value: "cpu", label: "CPU only", hint: "Much slower" },
  ];
  return (
    <Select.Root items={options} value={value} onValueChange={(v) => v && onDevice(v)}>
      <Select.Trigger className="field-trigger field-trigger-compact" aria-label="Graphics processor">
        <Select.Value className="source-trigger-value" />
        <Select.Icon className="source-trigger-chevron">
          <ChevronIcon width={14} height={14} />
        </Select.Icon>
      </Select.Trigger>
      <Select.Portal>
        <Select.Positioner className="select-positioner" sideOffset={6} alignItemWithTrigger={false}>
          <Select.Popup className="select-popup">
            <Select.List>
              {options.map((o) => (
                <Select.Item key={o.value} value={o.value} className="select-item select-item-plain">
                  <Select.ItemText className="select-item-text">{o.label}</Select.ItemText>
                  <span className="select-item-hint">{o.hint}</span>
                  <Select.ItemIndicator className="select-item-check">
                    <CheckIcon width={14} height={14} />
                  </Select.ItemIndicator>
                </Select.Item>
              ))}
            </Select.List>
          </Select.Popup>
        </Select.Positioner>
      </Select.Portal>
    </Select.Root>
  );
}

export function SettingsPanel(p: Props) {
  const asrModels = p.models.filter((m) => m.kind === "asr");
  const items = asrModels.map((m) => ({ value: m.id, label: m.name }));
  const stored = p.models.filter((m) => m.installed);
  const used = stored.reduce((n, m) => n + m.size, 0);
  // Required models are managed in the Models section; Storage lists the rest.
  const extra = stored.filter((m) => !p.required.some((r) => r.id === m.id));

  return (
    <div className="settings">
      <section>
        <h2>Display</h2>
        <div className="group">
          <Row label="Show Finnish original">
            <Toggle label="Show Finnish original" checked={p.view.showOriginal} onChange={(showOriginal) => p.onView({ ...p.view, showOriginal })} />
          </Row>
          <Row label="Text size">
            <RangeSlider
              label="Text size"
              value={p.view.fontSize}
              min={14}
              max={34}
              step={1}
              format={(v) => `${v}px`}
              onChange={(fontSize) => p.onView({ ...p.view, fontSize })}
            />
          </Row>
        </div>
      </section>

      <section>
        <h2>Listening</h2>
        <div className="group">
          <Row label="Live preview" hint="Show a draft while a sentence is still being spoken">
            <Toggle label="Live preview" checked={p.settings.livePreview} onChange={(livePreview) => p.onSettings({ ...p.settings, livePreview })} />
          </Row>
          <Row label="Sentence pause" hint="Silence that ends a sentence">
            <RangeSlider
              label="Sentence pause"
              value={p.settings.pauseMs}
              min={250}
              max={1500}
              step={50}
              format={(v) => `${(v / 1000).toFixed(2)}s`}
              onChange={(pauseMs) => p.onSettings({ ...p.settings, pauseMs })}
            />
          </Row>
          <Row label="Voice sensitivity" hint="Lower picks up quieter speech">
            <RangeSlider
              label="Voice sensitivity"
              value={Math.round((1 - p.settings.vadThreshold) * 100)}
              min={20}
              max={80}
              step={5}
              format={(v) => `${v}%`}
              onChange={(v) => p.onSettings({ ...p.settings, vadThreshold: 1 - v / 100 })}
            />
          </Row>
        </div>
      </section>

      <section>
        <h2>Shortcut</h2>
        <div className="group">
          <Row label="Show or hide Käännä" hint="Works from any app">
            <ShortcutRecorder value={p.toggleShortcut} onChange={p.onToggleShortcut} />
          </Row>
        </div>
      </section>

      <section>
        <h2>Models</h2>
        <Select.Root items={items} value={p.asrModel} onValueChange={(v) => v && p.onAsrModel(v)}>
          <Select.Trigger className="field-trigger">
            <Select.Value />
            <Select.Icon className="source-trigger-chevron">
              <ChevronIcon width={14} height={14} />
            </Select.Icon>
          </Select.Trigger>
          <Select.Portal>
            <Select.Positioner className="select-positioner" sideOffset={6} alignItemWithTrigger={false}>
              <Select.Popup className="select-popup">
                <Select.List>
                  {asrModels.map((m) => (
                    <Select.Item key={m.id} value={m.id} className="select-item select-item-plain">
                      <Select.ItemText className="select-item-text">{m.name}</Select.ItemText>
                      <span className="select-item-hint">{m.installed ? "Downloaded" : formatBytes(m.size)}</span>
                      <Select.ItemIndicator className="select-item-check">
                        <CheckIcon width={14} height={14} />
                      </Select.ItemIndicator>
                    </Select.Item>
                  ))}
                </Select.List>
              </Select.Popup>
            </Select.Positioner>
          </Select.Portal>
        </Select.Root>
        <div className="model-list">
          {p.required.map((m) => (
            <ModelRow
              key={m.id}
              model={m}
              progress={p.downloads[m.id]}
              onDownload={() => p.onDownload(m.id)}
              onCancel={() => p.onCancelDownload(m.id)}
              onDelete={() => p.onDeleteModel(m.id)}
            />
          ))}
        </div>
        {p.gpus.length > 0 && (
          <div className="group">
            <Row label="Graphics processor" hint={deviceHint(p.gpus, p.device)}>
              <ProcessorSelect gpus={p.gpus} device={p.device} onDevice={p.onDevice} />
            </Row>
          </div>
        )}
      </section>

      <section>
        <h2>
          Storage<span className="h2-note">{stored.length ? `${formatBytes(used)} on ${thisDevice}` : "No models downloaded"}</span>
        </h2>
        {extra.length > 0 && (
          <div className="model-list">
            {extra.map((m) => (
              <StoredModelRow key={m.id} model={m} onDelete={() => p.onDeleteModel(m.id)} />
            ))}
          </div>
        )}
      </section>

      <section className="settings-footer">
        <button className="btn btn-ghost btn-sm" onClick={p.onClear}>
          Clear transcript
        </button>
        <span className="settings-note">Whisper Large v3 Finnish · OPUS-MT · Silero VAD — all local</span>
      </section>
    </div>
  );
}
