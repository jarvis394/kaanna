import { useEffect, useState } from "react";

/**
 * Keeps the last non-null value around for `exitMs` after `value` becomes null, so it can animate out.
 * Returns the value to render (null once gone) and whether it is currently leaving.
 */
export function usePresence<T>(value: T | null, exitMs: number): [T | null, boolean] {
  const [last, setLast] = useState(value);

  useEffect(() => {
    if (value != null) {
      setLast(value);
      return;
    }
    const t = window.setTimeout(() => setLast(null), exitMs);
    return () => window.clearTimeout(t);
  }, [value, exitMs]);

  return [value ?? last, value == null && last != null];
}
