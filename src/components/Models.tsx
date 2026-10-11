import { Progress } from "@base-ui/react/progress";
import { useState } from "react";
import { formatBytes, type DownloadProgress, type Model } from "../lib/api";
import { CheckIcon, DownloadIcon } from "./Icons";

export type Downloads = Record<string, DownloadProgress | undefined>;

const KIND_LABEL = { asr: "Speech recognition", mt: "Translation", vad: "Voice detection" } as const;

export function ModelRow({ model, progress, onDownload }: { model: Model; progress?: DownloadProgress; onDownload?: () => void }) {
  const pct = progress && progress.total > 0 ? (progress.received / progress.total) * 100 : null;
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
          <span className="badge" data-celebrate={justFinished || undefined}>
            <CheckIcon width={12} height={12} /> Ready
          </span>
        ) : pct !== null ? (
          <span className="badge badge-muted">{Math.floor(pct)}%</span>
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

interface SetupProps {
  required: Model[];
  downloads: Downloads;
  error: string | null;
  onDownloadAll: () => void;
}

/** First-run screen: fetch the models once, then everything is offline. */
export function Setup({ required, downloads, error, onDownloadAll }: SetupProps) {
  const missing = required.filter((m) => !m.installed);
  const busy = missing.some((m) => downloads[m.id]);
  const total = missing.reduce((n, m) => n + m.size, 0);
  return (
    <div className="setup">
      <div className="setup-head">
        <h1>Set up on-device translation</h1>
        <p>
          Käännä listens and translates entirely on this Mac. The models are downloaded once from Hugging Face — after that,
          nothing leaves your computer.
        </p>
      </div>
      <div className="model-list">
        {required.map((m) => (
          <ModelRow key={m.id} model={m} progress={downloads[m.id]} />
        ))}
      </div>
      {error && <div className="inline-error">{error}</div>}
      <button className="btn btn-primary" disabled={busy} onClick={onDownloadAll}>
        {busy ? "Downloading…" : `Download ${formatBytes(total)}`}
      </button>
    </div>
  );
}
