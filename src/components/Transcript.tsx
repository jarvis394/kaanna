import { useLayoutEffect, useRef, useState } from "react";
import type { Utterance } from "../lib/api";
import { ArrowDownIcon } from "./Icons";

interface Props {
  lines: Utterance[];
  showOriginal: boolean;
  fontSize: number;
  listening: boolean;
}

/** Large text wants tighter tracking and leading; small text a little looser. */
function typeMetrics(fontSize: number) {
  const t = Math.min(1, Math.max(0, (fontSize - 14) / 20));
  return { tracking: `${(-0.004 - 0.018 * t).toFixed(4)}em`, leading: (1.36 - 0.14 * t).toFixed(3) };
}

export function Transcript({ lines, showOriginal, fontSize, listening }: Props) {
  const scroller = useRef<HTMLDivElement>(null);
  const following = useRef(true);
  const seeking = useRef(false);
  const [detached, setDetached] = useState(false);
  // Lines already on screen when this view mounts (e.g. coming back from Settings) shouldn't replay their entrance.
  const [settled] = useState(() => new Set(lines.map((l) => l.id)));

  // Follow new lines unless the user scrolled up to read something.
  useLayoutEffect(() => {
    const el = scroller.current;
    if (el && following.current) el.scrollTop = el.scrollHeight;
  }, [lines, showOriginal, fontSize]);

  const backToLive = () => {
    const el = scroller.current;
    if (!el) return;
    following.current = true;
    seeking.current = true;
    setDetached(false);
    el.scrollTo({ top: el.scrollHeight, behavior: "smooth" });
  };

  if (lines.length === 0) {
    return (
      <div className="transcript-empty" data-tauri-drag-region>
        {listening ? (
          <>
            <div className="empty-title">Listening for Finnish…</div>
            <div className="empty-sub">Translations appear here as soon as someone speaks.</div>
          </>
        ) : (
          <>
            <div className="empty-title">Ready to translate</div>
            <div className="empty-sub">Pick an audio source and press start.</div>
          </>
        )}
      </div>
    );
  }

  const { tracking, leading } = typeMetrics(fontSize);

  return (
    <div className="transcript-wrap">
      <div
        ref={scroller}
        className="transcript"
        data-detached={detached || undefined}
        data-tauri-drag-region
        style={{ "--font-size": `${fontSize}px`, "--tracking": tracking, "--leading": leading } as React.CSSProperties}
        onScroll={(e) => {
          const el = e.currentTarget;
          const atEnd = el.scrollHeight - el.scrollTop - el.clientHeight < 24;
          // Ignore the in-between positions of our own smooth scroll back to the end.
          if (seeking.current) {
            if (atEnd) seeking.current = false;
            return;
          }
          following.current = atEnd;
          setDetached(!atEnd);
        }}
      >
        <div className="transcript-inner" data-tauri-drag-region>
          {lines.map((u, i) => {
            const age = lines.length - 1 - i;
            return (
              <div
                key={u.id}
                className="line"
                data-final={u.isFinal}
                data-age={Math.min(age, 3)}
                data-settled={settled.has(u.id) || undefined}
                title={u.isFinal ? `${(u.audioMs / 1000).toFixed(1)}s of speech · recognised in ${u.asrMs} ms · translated in ${u.mtMs} ms` : undefined}
              >
                <p className="line-en">{u.translation || u.source}</p>
                {showOriginal && u.translation && <p className="line-fi">{u.source}</p>}
              </div>
            );
          })}
        </div>
      </div>
      {detached && (
        <button className="live-pill" onClick={backToLive}>
          <ArrowDownIcon width={12} height={12} strokeWidth={2.2} />
          Live
        </button>
      )}
    </div>
  );
}
