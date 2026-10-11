// Apple-style activity indicator, from jarvis394/valley (packages/ui/src/Spinner).
// The 12 bars never move; each fades in turn, so there is no rotation pivot to get wrong.
const BARS = Array.from({ length: 12 }, (_, i) => i);

export function Spinner({ size = 16 }: { size?: number }) {
  return (
    <span className="spinner" style={{ "--spinner-size": `${size}px` } as React.CSSProperties} aria-hidden>
      <span className="spinner-inner">
        {BARS.map((i) => (
          <span
            key={i}
            className="spinner-bar"
            style={{ transform: `rotate(${i * 30}deg) translate(146%)`, animationDelay: `${-1.2 + i * 0.1}s` }}
          />
        ))}
      </span>
    </span>
  );
}
