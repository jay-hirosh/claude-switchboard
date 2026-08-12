import { render, screen } from '@testing-library/react';
import { describe, it, expect } from 'vitest';
import type { LiveSessionInfo } from '../../lib/types';
import { NowRunningSection } from '../NowRunningSection';

function session(overrides: Partial<LiveSessionInfo>): LiveSessionInfo {
  const now = Math.floor(Date.now() / 1000);
  return {
    session_id: 's1',
    source_file: '-proj/s1.jsonl',
    project: 'my-project',
    model: 'claude-opus-5',
    total_tokens: 12000,
    total_cost_usd: 1.84,
    context_tokens: 5000,
    first_seen: now - 12 * 60,
    last_activity: now,
    ...overrides,
  };
}

describe('NowRunningSection', () => {
  it('renders nothing when there are no live sessions', () => {
    const { container } = render(<NowRunningSection sessions={[]} />);
    expect(container.firstChild).toBeNull();
  });

  it('renders a row per session with project, model, cost, and elapsed', () => {
    render(
      <NowRunningSection
        sessions={[
          session({ session_id: 'a', project: 'proj-a', total_cost_usd: 1.84, first_seen: Math.floor(Date.now() / 1000) - 12 * 60 }),
          session({ session_id: 'b', project: 'proj-b', total_cost_usd: 0.02 }),
        ]}
      />,
    );
    expect(screen.getByText('proj-a')).toBeTruthy();
    expect(screen.getByText('proj-b')).toBeTruthy();
    expect(screen.getByText('$1.84')).toBeTruthy();
    // Both sessions carry the helper's default 12m-ago first_seen (only
    // session 'a' overrides it, to the same computed value), so both rows
    // render "12m" — assert on the set rather than a single unique match.
    expect(screen.getAllByText(/12m/).length).toBeGreaterThan(0);
  });

  it('caps at 3 rows and shows a "+N more" line beyond that', () => {
    const sessions = Array.from({ length: 5 }, (_, i) => session({ session_id: `s${i}`, project: `proj-${i}` }));
    render(<NowRunningSection sessions={sessions} />);
    expect(screen.getByText('proj-0')).toBeTruthy();
    expect(screen.getByText('proj-1')).toBeTruthy();
    expect(screen.getByText('proj-2')).toBeTruthy();
    expect(screen.queryByText('proj-3')).toBeNull();
    expect(screen.getByText(/\+2 more/)).toBeTruthy();
  });
});
