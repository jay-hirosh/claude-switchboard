import { render, screen, fireEvent } from '@testing-library/react';
import { describe, it, expect, vi } from 'vitest';
import type { SessionSummary } from '../../lib/generated/bindings';
import { SessionRow } from '../SessionRow';

function session(over: Partial<SessionSummary> = {}): SessionSummary {
  return {
    session_id: '029a3e04-fa36',
    cwd: '/Users/me/Developer/claude-switchboard',
    project_name: 'claude-switchboard',
    git_branch: 'main',
    title: 'Plan custom model swapping feature',
    recap: 'Goal: add provider support. Next: review the spec.',
    asked: 'we need a to plan for another major feature',
    left_off: 'what about spec B?',
    touched_files: ['design.md', 'plan.md'],
    touched_overflow: 3,
    model: 'glm-5.2',
    peak_context_tokens: 410_000,
    turns: 12,
    started_at: '2026-07-28T23:07:00Z',
    ended_at: '2026-07-29T01:14:00Z',
    ...over,
  };
}

describe('SessionRow', () => {
  it('shows title, branch and model when collapsed', () => {
    render(
      <SessionRow session={session()} expanded={false} onToggle={vi.fn()} onResume={vi.fn()} />,
    );
    expect(screen.getByText('Plan custom model swapping feature')).toBeTruthy();
    expect(screen.getByText(/main/)).toBeTruthy();
    expect(screen.getByText('glm-5.2')).toBeTruthy();
  });

  // Grouped mode puts the project in the section header directly above, so
  // repeating it on every row is noise. Search flattens the grouping and the
  // project becomes the missing context, so it comes back.
  it('omits the project when grouped and shows it when searching', () => {
    const { rerender } = render(
      <SessionRow session={session()} expanded={false} onToggle={vi.fn()} onResume={vi.fn()} />,
    );
    expect(screen.queryByText(/claude-switchboard/)).toBeNull();

    rerender(
      <SessionRow
        session={session()}
        expanded={false}
        showProject
        onToggle={vi.fn()}
        onResume={vi.fn()}
      />,
    );
    expect(screen.getByText(/claude-switchboard/)).toBeTruthy();
  });

  it('hides the recap until expanded', () => {
    const { rerender } = render(
      <SessionRow session={session()} expanded={false} onToggle={vi.fn()} onResume={vi.fn()} />,
    );
    expect(screen.queryByText(/we need a to plan/)).toBeNull();
    rerender(<SessionRow session={session()} expanded onToggle={vi.fn()} onResume={vi.fn()} />);
    expect(screen.getByText(/we need a to plan/)).toBeTruthy();
    expect(screen.getByText(/what about spec B/)).toBeTruthy();
  });

  it('shows the recap first when present', () => {
    render(<SessionRow session={session()} expanded onToggle={vi.fn()} onResume={vi.fn()} />);
    expect(screen.getByText(/Goal: add provider support/)).toBeTruthy();
  });

  it('omits the Recap row when the session has none', () => {
    render(
      <SessionRow
        session={session({ recap: null })}
        expanded
        onToggle={vi.fn()}
        onResume={vi.fn()}
      />,
    );
    expect(screen.queryByText(/^recap$/i)).toBeNull();
  });

  it('shows touched files with an overflow count', () => {
    render(<SessionRow session={session()} expanded onToggle={vi.fn()} onResume={vi.fn()} />);
    expect(screen.getByText('design.md')).toBeTruthy();
    expect(screen.getByText(/\+3 more/)).toBeTruthy();
  });

  it('omits the Touched row entirely when nothing was touched', () => {
    render(
      <SessionRow
        session={session({ touched_files: [], touched_overflow: 0 })}
        expanded
        onToggle={vi.fn()}
        onResume={vi.fn()}
      />,
    );
    expect(screen.queryByText(/touched/i)).toBeNull();
  });

  it('omits Left off when absent', () => {
    render(
      <SessionRow
        session={session({ left_off: null })}
        expanded
        onToggle={vi.fn()}
        onResume={vi.fn()}
      />,
    );
    expect(screen.queryByText(/left off/i)).toBeNull();
  });

  // The assertion used to demand the literal word "unknown", contradicting the
  // test's own name: a transcript with no assistant `model` is missing data,
  // not a model called "unknown", and printing it put a full-weight chip in
  // the column where every other row carries real information.
  it('renders an unknown-model session without a badge', () => {
    render(
      <SessionRow
        session={session({ model: null })}
        expanded={false}
        onToggle={vi.fn()}
        onResume={vi.fn()}
      />,
    );
    expect(screen.queryByText(/unknown/i)).toBeNull();
    expect(screen.getByText('Plan custom model swapping feature')).toBeTruthy();
  });

  // Claude Code strips the [1m] suffix before writing a transcript, so the
  // configured window is not recorded. Exceeding the 200K standard window is
  // the only surviving evidence that a session ran on a 1M one.
  it('infers a 1M window when peak context exceeds the standard 200K', () => {
    render(
      <SessionRow
        session={session({ peak_context_tokens: 410_000 })}
        expanded
        onToggle={vi.fn()}
        onResume={vi.fn()}
      />,
    );
    expect(screen.getByText(/410\.0K of 1M/)).toBeTruthy();
  });

  it('assumes the standard window when peak context stays under it', () => {
    render(
      <SessionRow
        session={session({ peak_context_tokens: 57_000 })}
        expanded
        onToggle={vi.fn()}
        onResume={vi.fn()}
      />,
    );
    expect(screen.getByText(/57\.0K of 200K/)).toBeTruthy();
  });

  it('omits the context readout when no turn reported usage', () => {
    render(
      <SessionRow
        session={session({ peak_context_tokens: null })}
        expanded
        onToggle={vi.fn()}
        onResume={vi.fn()}
      />,
    );
    expect(screen.queryByText(/ of (1M|200K)/)).toBeNull();
  });

  it('calls onResume with the session id', () => {
    const onResume = vi.fn();
    render(<SessionRow session={session()} expanded onToggle={vi.fn()} onResume={onResume} />);
    fireEvent.click(screen.getByRole('button', { name: /resume/i }));
    expect(onResume).toHaveBeenCalledWith(session().session_id);
  });
});
