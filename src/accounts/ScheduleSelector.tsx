// src/accounts/ScheduleSelector.tsx
import { Plus, Trash2 } from "lucide-react";

export interface HhMm { hour: number; minute: number; }
export type Schedule =
  | { type: "Off" }
  | { type: "Every5h"; anchor: HhMm }
  | { type: "Custom"; times: HhMm[] };

export interface WarmupSuggestion {
  anchor: HhMm;
  activeDays: number;
}

interface Props {
  value: Schedule;
  onChange: (next: Schedule) => void;
  /** Median first-activity time observed over recent history. Omit or pass
   * null when there isn't enough history yet — the suggestion line simply
   * doesn't render, same as any other absent-data case in this app. */
  suggestion?: WarmupSuggestion | null;
}

const fmtHm = (h: HhMm) =>
  `${String(h.hour).padStart(2, "0")}:${String(h.minute).padStart(2, "0")}`;

const parseHm = (s: string): HhMm | null => {
  const m = /^(\d{1,2}):(\d{2})$/.exec(s.trim());
  if (!m) return null;
  const hour = Number(m[1]);
  const minute = Number(m[2]);
  if (hour < 0 || hour > 23 || minute < 0 || minute > 59) return null;
  return { hour, minute };
};

export function ScheduleSelector({ value, onChange, suggestion }: Props) {
  const showSuggestion =
    !!suggestion &&
    !(
      value.type === "Every5h" &&
      value.anchor.hour === suggestion.anchor.hour &&
      value.anchor.minute === suggestion.anchor.minute
    );

  return (
    <div className="space-y-2 text-[12px]">
      {showSuggestion && suggestion && (
        <div
          className="flex items-center gap-2 text-neutral-400"
          title={`Based on ${suggestion.activeDays} active days`}
        >
          <span>You usually start around {fmtHm(suggestion.anchor)} —</span>
          <button
            type="button"
            onClick={() => onChange({ type: "Every5h", anchor: suggestion.anchor })}
            className="px-2 py-0.5 rounded bg-neutral-800/40 hover:bg-neutral-800/60 text-neutral-300 text-[11px]"
          >
            Apply
          </button>
        </div>
      )}

      <fieldset className="flex items-center gap-3">
        <label className="flex items-center gap-2">
          <input
            type="radio"
            checked={value.type === "Off"}
            onChange={() => onChange({ type: "Off" })}
          />
          Off
        </label>
        <label className="flex items-center gap-2">
          <input
            type="radio"
            checked={value.type === "Every5h"}
            onChange={() =>
              onChange({ type: "Every5h", anchor: { hour: 6, minute: 0 } })
            }
          />
          Every 5h
        </label>
        <label className="flex items-center gap-2">
          <input
            type="radio"
            checked={value.type === "Custom"}
            onChange={() => onChange({ type: "Custom", times: [] })}
          />
          Custom
        </label>
      </fieldset>

      {value.type === "Every5h" && (
        <div className="ml-5 flex items-center gap-2">
          <span className="text-neutral-400">Anchor:</span>
          <input
            type="time"
            className="bg-neutral-800/50 px-2 py-0.5 rounded text-neutral-100"
            value={fmtHm(value.anchor)}
            onChange={(e) => {
              const h = parseHm(e.target.value);
              if (h)
                onChange({ type: "Every5h", anchor: h });
            }}
          />
        </div>
      )}

      {value.type === "Custom" && (
        <div className="ml-5 space-y-1">
          {value.times.map((t, idx) => (
            <div key={idx} className="flex items-center gap-2">
              <input
                type="time"
                className="bg-neutral-800/50 px-2 py-0.5 rounded text-neutral-100"
                value={fmtHm(t)}
                onChange={(e) => {
                  const h = parseHm(e.target.value);
                  if (!h) return;
                  const next = [...value.times];
                  next[idx] = h;
                  onChange({ type: "Custom", times: next });
                }}
              />
              <button
                type="button"
                aria-label="Remove time"
                onClick={() => {
                  const next = value.times.filter((_, i) => i !== idx);
                  onChange({ type: "Custom", times: next });
                }}
                className="text-neutral-400 hover:text-neutral-200"
              >
                <Trash2 className="w-3.5 h-3.5" />
              </button>
            </div>
          ))}
          <button
            type="button"
            onClick={() =>
              onChange({
                type: "Custom",
                times: [...value.times, { hour: 9, minute: 0 }],
              })
            }
            className="flex items-center gap-1 px-2 py-0.5 rounded bg-neutral-800/40 hover:bg-neutral-800/60 text-neutral-300 text-[11px]"
          >
            <Plus className="w-3 h-3" />
            Add time
          </button>
        </div>
      )}
    </div>
  );
}
