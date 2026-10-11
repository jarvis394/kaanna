import { Select } from "@base-ui/react/select";
import type { Source, Status } from "../lib/api";
import { CheckIcon, ChevronIcon, MicIcon, SpeakerIcon } from "./Icons";

interface Props {
  sources: Source[];
  value: string | null;
  onChange: (id: string) => void;
  onOpen: () => void;
  /** The trigger icon doubles as the status light: green while listening. */
  state: Status["state"];
}

function SourceIcon({ kind }: { kind: Source["kind"] }) {
  return kind === "system" ? <SpeakerIcon /> : <MicIcon />;
}

export function SourceSelect({ sources, value, onChange, onOpen, state }: Props) {
  const items = sources.map((s) => ({ value: s.id, label: s.name }));
  const current = sources.find((s) => s.id === value);
  return (
    <Select.Root
      items={items}
      value={value}
      onValueChange={(v) => v && onChange(v)}
      onOpenChange={(open) => open && onOpen()}
    >
      <Select.Trigger className="source-trigger" aria-label="Audio source">
        <span className="source-trigger-icon" data-state={state}>
          {current && <SourceIcon kind={current.kind} />}
        </span>
        <Select.Value className="source-trigger-value" placeholder="Choose audio source" />
        <Select.Icon className="source-trigger-chevron">
          <ChevronIcon width={14} height={14} />
        </Select.Icon>
      </Select.Trigger>
      <Select.Portal>
        <Select.Positioner className="select-positioner" sideOffset={6} alignItemWithTrigger={false}>
          <Select.Popup className="select-popup">
            <Select.List>
              {sources.map((s, i) => (
                <div key={s.id}>
                  {i > 0 && sources[i - 1].kind !== s.kind && <Select.Separator className="select-separator" />}
                  <Select.Item value={s.id} className="select-item">
                    <span className="select-item-icon">
                      <SourceIcon kind={s.kind} />
                    </span>
                    <Select.ItemText className="select-item-text">{s.name}</Select.ItemText>
                    {s.kind === "system" && <span className="select-item-hint">Everything you hear</span>}
                    {s.isDefault && <span className="select-item-hint">Default</span>}
                    <Select.ItemIndicator className="select-item-check">
                      <CheckIcon width={14} height={14} />
                    </Select.ItemIndicator>
                  </Select.Item>
                </div>
              ))}
            </Select.List>
          </Select.Popup>
        </Select.Positioner>
      </Select.Portal>
    </Select.Root>
  );
}
