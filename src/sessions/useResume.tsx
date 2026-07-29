// Placeholder — replaced in Task 8 with real provider resolution + picker.
import type { SessionSummary } from '../lib/generated/bindings';

export function useResume() {
  return {
    resume: (_: SessionSummary) => {},
    dialog: null as React.ReactNode,
    notice: null as string | null,
  };
}
