import { Progress } from "@base-ui/react/progress";
import { useEffect, useState } from "react";
import { formatBytes, thisDevice, type DownloadProgress, type Model } from "../lib/api";
import { CheckIcon, DownloadIcon } from "./Icons";

export type Downloads = Record<string, DownloadProgress | undefined>;

const KIND_LABEL = { asr: "Speech recognition", mt: "Translation", vad: "Voice detection" } as const;

/** Two-step Delete: the first click arms it, the second deletes. Disarms itself after a moment. */
function DeleteButton({ onDelete }: { onDelete: () => void }) {
  const [armed, setArmed] = useState(false);
  useEffect(() => {
    if (!armed) return;
    const t = window.setTimeout(() => setArmed(false), 3000);
    return () => clearTimeout(t);
  }, [armed]);
  return (
    <button
      className="btn btn-ghost btn-sm"
      data-danger={armed || undefined}
      onClick={() => (armed ? onDelete() : setArmed(true))}
      onBlur={() => setArmed(false)}
    >
      {armed ? "Confirm delete" : "Delete"}
    </button>
  );
}

interface RowProps {
  model: Model;
  progress?: DownloadProgress;
  onDownload?: () => void;
  onCancel?: () => void;
  onDelete?: () => void;
}

export function ModelRow({ model, progress, onDownload, onCancel, onDelete }: RowProps) {
  const pct = progress ? (progress.total > 0 ? (progress.received / progress.total) * 100 : 0) : null;
  // Celebrate only a download that finishes while this row is on screen, not models that were already there.
  const [installedAtMount] = useState(model.installed);
  const justFinished = model.installed && !installedAtMount;
  return (
    <div className="model-row" data-installed={model.installed}>
      <div className="model-row-text">
        <div className="model-row-kind">{KIND_LABEL[model.kind]}</div>
        <div className="model-row-name">{model.name}</div>
        {pct !== null && !model.installed ? (
          <Progress.Root className="progress" value={pct}>
            <Progress.Track className="progress-track">
              <Progress.Indicator className="progress-indicator" />
            </Progress.Track>
            <span className="progress-label">
              {formatBytes(progress!.received)} / {formatBytes(progress!.total)}
            </span>
          </Progress.Root>
        ) : (
          <div className="model-row-desc">{model.description}</div>
        )}
      </div>
      <div className="model-row-action">
        {model.installed ? (
          <>
            <span className="badge badge-ready" data-celebrate={justFinished || undefined}>
              <CheckIcon width={12} height={12} /> Ready
            </span>
            {onDelete && <DeleteButton onDelete={onDelete} />}
          </>
        ) : pct !== null ? (
          <>
            <span className="badge badge-muted">{Math.floor(pct)}%</span>
            {onCancel && (
              <button className="btn btn-ghost btn-sm" onClick={onCancel}>
                Cancel
              </button>
            )}
          </>
        ) : onDownload ? (
          <button className="btn btn-ghost btn-sm" onClick={onDownload}>
            <DownloadIcon width={13} height={13} /> {formatBytes(model.size)}
          </button>
        ) : (
          <span className="badge badge-muted">{formatBytes(model.size)}</span>
        )}
      </div>
    </div>
  );
}

/** Settings → Storage: a downloaded model that isn't in use, with Delete. */
export function StoredModelRow({ model, onDelete }: { model: Model; onDelete: () => void }) {
  return (
    <div className="model-row">
      <div className="model-row-text">
        <div className="model-row-kind">{KIND_LABEL[model.kind]}</div>
        <div className="model-row-name">{model.name}</div>
        <div className="model-row-desc">{formatBytes(model.size)}</div>
      </div>
      <div className="model-row-action">
        <DeleteButton onDelete={onDelete} />
      </div>
    </div>
  );
}

interface SetupProps {
  required: Model[];
  downloads: Downloads;
  error: string | null;
  onDownload: (id: string) => void;
  onCancel: (id: string) => void;
}

/** First-run screen (and the way back if a model goes missing): fetch the models once, then everything is offline. */
export function Setup({ required, downloads, error, onDownload, onCancel }: SetupProps) {
  const missing = required.filter((m) => !m.installed);
  const pending = missing.filter((m) => !downloads[m.id]);
  const busy = pending.length < missing.length;
  const pendingSize = pending.reduce((n, m) => n + m.size, 0);
  // Small ones first, so their progress shows up right away.
  const downloadPending = () => [...pending].sort((a, b) => a.size - b.size).forEach((m) => onDownload(m.id));
  const cancelAll = () => missing.filter((m) => downloads[m.id]).forEach((m) => onCancel(m.id));
  return (
    <div className="setup">
      <div className="setup-head">
        <h1>Set up on-device translation</h1>
        <p>
          Käännä listens and translates entirely on {thisDevice}. The models are downloaded once from Hugging Face — after
          that, nothing leaves your computer.
        </p>
      </div>
      <div className="model-list">
        {required.map((m) => (
          <ModelRow
            key={m.id}
            model={m}
            progress={downloads[m.id]}
            onDownload={() => onDownload(m.id)}
            onCancel={() => onCancel(m.id)}
          />
        ))}
      </div>
      {error && <div className="inline-error">{error}</div>}
      {pending.length > 0 ? (
        <button className="btn btn-primary" onClick={downloadPending}>
          {busy ? "Download the rest" : "Download all"} · {formatBytes(pendingSize)}
        </button>
      ) : (
        busy && (
          <button className="btn btn-ghost" onClick={cancelAll}>
            Cancel all
          </button>
        )
      )}
    </div>
  );
}
