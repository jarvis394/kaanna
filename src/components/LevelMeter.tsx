import { useEffect, useRef } from "react";
import { on } from "../lib/api";

const BARS = [0.55, 0.85, 1, 0.75, 0.5];

/** Tiny live input meter. Updates the DOM directly to avoid re-rendering at ~16 Hz. */
export function LevelMeter({ active }: { active: boolean }) {
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    let smoothed = 0;
    const unlisten = on("audio-level", ({ level, speaking }) => {
      smoothed = Math.max(level, smoothed * 0.6);
      const el = ref.current;
      if (!el) return;
      el.style.setProperty("--level", smoothed.toFixed(3));
      el.dataset.speaking = String(speaking);
    });
    return () => {
      unlisten.then((f) => f());
    };
  }, []);

  useEffect(() => {
    if (!active) ref.current?.style.setProperty("--level", "0");
  }, [active]);

  return (
    <div ref={ref} className="level-meter" data-active={active} aria-hidden>
      {BARS.map((w, i) => (
        <span key={i} style={{ "--w": w } as React.CSSProperties} />
      ))}
    </div>
  );
}
