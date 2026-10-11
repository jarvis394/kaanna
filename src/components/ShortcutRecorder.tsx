import { useState, type KeyboardEvent } from "react";
import { api } from "../lib/api";

export const DEFAULT_TOGGLE_SHORTCUT = "Command+Alt+Backslash";

// Accelerator tokens in macOS menu order: ⌃⌥⇧⌘.
const MODIFIERS = [
  ["Control", "⌃"],
  ["Alt", "⌥"],
  ["Shift", "⇧"],
  ["Command", "⌘"],
] as const;

const KEY_LABELS: Record<string, string> = {
  Backslash: "\\",
  Slash: "/",
  Comma: ",",
  Period: ".",
  Semicolon: ";",
  Quote: "'",
  BracketLeft: "[",
  BracketRight: "]",
  Minus: "-",
  Equal: "=",
  Backquote: "`",
  Space: "Space",
  Enter: "↩",
  Tab: "⇥",
  Backspace: "⌫",
  Delete: "⌦",
  ArrowUp: "↑",
  ArrowDown: "↓",
  ArrowLeft: "←",
  ArrowRight: "→",
};

/** "Command+Alt+Backslash" → "⌥⌘\" */
export function formatShortcut(accelerator: string) {
  const parts = accelerator.split("+");
  const key = parts.pop() ?? "";
  const mods = MODIFIERS.filter(([name]) => parts.includes(name)).map(([, glyph]) => glyph);
  const label = KEY_LABELS[key] ?? key.replace(/^Key|^Digit/, "");
  return mods.join("") + label;
}

function heldModifiers(e: KeyboardEvent) {
  return MODIFIERS.filter(([name]) => ({ Control: e.ctrlKey, Alt: e.altKey, Shift: e.shiftKey, Command: e.metaKey })[name]);
}

interface Props {
  value: string;
  onChange: (accelerator: string) => void;
}

/** Click, then press the new combination. Esc or clicking away keeps the old one. */
export function ShortcutRecorder({ value, onChange }: Props) {
  const [recording, setRecording] = useState(false);
  const [held, setHeld] = useState("");
  const [error, setError] = useState<string | null>(null);

  const begin = () => {
    setError(null);
    setHeld("");
    setRecording(true);
    // Otherwise pressing the current shortcut would hide the window mid-recording.
    api.setToggleShortcut(null);
  };

  const finish = async (next: string | null) => {
    setRecording(false);
    setHeld("");
    try {
      await api.setToggleShortcut(next ?? value);
      setError(null);
      if (next) onChange(next);
    } catch (e) {
      setError(String(e));
      await api.setToggleShortcut(value).catch(() => {});
    }
  };

  const onKeyDown = (e: KeyboardEvent) => {
    if (!recording) {
      // Start on key-down; the key-up "click" is suppressed below.
      if (e.key === "Enter" || e.key === " ") {
        e.preventDefault();
        begin();
      }
      return;
    }
    e.preventDefault();
    e.stopPropagation();
    const mods = heldModifiers(e);
    if (e.key === "Escape" && mods.length === 0) return void finish(null);
    if (["Meta", "Alt", "Shift", "Control"].includes(e.key)) {
      setHeld(mods.map(([, glyph]) => glyph).join(""));
      return;
    }
    if (!e.metaKey && !e.ctrlKey && !e.altKey) {
      setError("Include ⌘, ⌥ or ⌃");
      return;
    }
    finish([...mods.map(([name]) => name), e.code].join("+"));
  };

  return (
    <div className="shortcut">
      <button
        className="shortcut-field"
        data-recording={recording || undefined}
        onClick={() => (recording ? finish(null) : begin())}
        onKeyDown={onKeyDown}
        onKeyUp={(e) => {
          // Keyboard activation happens on key-down; a native key-up click would toggle it straight back.
          e.preventDefault();
          if (recording) setHeld(heldModifiers(e).map(([, glyph]) => glyph).join(""));
        }}
        onBlur={() => recording && finish(null)}
        aria-label="Show or hide shortcut"
      >
        {recording ? held || "Type shortcut…" : formatShortcut(value)}
      </button>
      {error && <span className="shortcut-error">{error}</span>}
    </div>
  );
}
