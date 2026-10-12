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

interface Option {
  value: string;
  label: string;
  hint: string;
}

/** A compact menu button that fits in a settings row; each item shows a hint on the right. */
function RowSelect({ label, options, value, onChange }: { label: string; options: Option[]; value: string; onChange: (v: string) => void }) {
  return (
    <Select.Root items={options} value={value} onValueChange={(v) => v && onChange(v)}>
      <Select.Trigger className="field-trigger field-trigger-compact" aria-label={label}>
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

function processorOptions(gpus: Gpu[]): Option[] {
  return [
    { value: "auto", label: "Automatic", hint: "Prefers dedicated" },
    ...gpus.map((g) => ({ value: `gpu:${g.index}`, label: g.name, hint: gpuHint(g) })),
    { value: "cpu", label: "CPU only", hint: "Much slower" },
  ];
}

export function SettingsPanel(p: Props) {
  const asrModels = p.models.filter((m) => m.kind === "asr");
  const modelOptions = asrModels.map((m) => ({ value: m.id, label: m.name, hint: m.installed ? "Downloaded" : formatBytes(m.size) }));
  // A GPU that's gone (e.g. an unplugged eGPU) falls back to automatic, as in the backend.
  const device = p.device === "cpu" || p.gpus.some((g) => `gpu:${g.index}` === p.device) ? p.device : "auto";
  const stored = p.models.filter((m) => m.installed);
  const used = stored.reduce((n, m) => n + m.size, 0);
  // Downloaded models that aren't in use, listed in Storage after the required ones.
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
          <Row label="Show or hide Käännä">
            <ShortcutRecorder value={p.toggleShortcut} onChange={p.onToggleShortcut} />
          </Row>
        </div>
      </section>

      <section>
        <h2>Models</h2>
        <div className="group">
          <Row label="Selected model" hint="Transcribes the speech in the audio">
            <RowSelect label="Selected model" options={modelOptions} value={p.asrModel} onChange={p.onAsrModel} />
          </Row>
          {p.gpus.length > 0 && (
            <Row label="Graphics processor" hint={deviceHint(p.gpus, device)}>
              <RowSelect label="Graphics processor" options={processorOptions(p.gpus)} value={device} onChange={p.onDevice} />
            </Row>
          )}
        </div>
      </section>

      <section>
        <h2>
          Storage<span className="h2-note">{stored.length ? `${formatBytes(used)} on ${thisDevice}` : "No models downloaded"}</span>
        </h2>
        {/* The models in use first, with download controls; then any others still on disk. */}
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
          {extra.map((m) => (
            <StoredModelRow key={m.id} model={m} onDelete={() => p.onDeleteModel(m.id)} />
          ))}
        </div>
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
