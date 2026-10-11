import { useLayoutEffect, useRef } from "react";
import type { Utterance } from "../lib/api";

interface Props {
  lines: Utterance[];
  showOriginal: boolean;
  fontSize: number;
  listening: boolean;
}

export function Transcript({ lines, showOriginal, fontSize, listening }: Props) {
  const scroller = useRef<HTMLDivElement>(null);
  const pinned = useRef(true);

  // Follow new lines unless the user scrolled up to read something.
  useLayoutEffect(() => {
    const el = scroller.current;
    if (el && pinned.current) el.scrollTop = el.scrollHeight;
  }, [lines, showOriginal, fontSize]);

  if (lines.length === 0) {
    return (
      <div className="transcript-empty">
        {listening ? (
          <>
            <div className="empty-title">Listening for Finnish…</div>
            <div className="empty-sub">Translations appear here as soon as someone speaks.</div>
          </>
        ) : (
          <>
            <div className="empty-title">Ready when you are</div>
            <div className="empty-sub">Pick an audio source and press start. Everything runs on this Mac.</div>
          </>
        )}
      </div>
    );
  }

  return (
    <div
      ref={scroller}
      className="transcript"
      style={{ "--font-size": `${fontSize}px` } as React.CSSProperties}
      onScroll={(e) => {
        const el = e.currentTarget;
        pinned.current = el.scrollHeight - el.scrollTop - el.clientHeight < 24;
      }}
    >
      <div className="transcript-inner">
        {lines.map((u, i) => {
          const age = lines.length - 1 - i;
          return (
            <div
              key={u.id}
              className="line"
              data-final={u.isFinal}
              data-age={Math.min(age, 3)}
              title={u.isFinal ? `${(u.audioMs / 1000).toFixed(1)}s of speech · recognised in ${u.asrMs} ms · translated in ${u.mtMs} ms` : undefined}
            >
              <p className="line-en">{u.translation || u.source}</p>
              {showOriginal && u.translation && <p className="line-fi">{u.source}</p>}
            </div>
          );
        })}
      </div>
    </div>
  );
}
