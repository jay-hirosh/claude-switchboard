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
    total_tokens: 12_345,
    total_cost_usd: 1.23,
    permission_mode: null,
    cwd_exists: true,
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

  // Resuming cds into session.cwd, and Claude Code locates a transcript only
  // through the directory it is launched in — so a deleted project folder
  // makes resume impossible, not just likely to fail.
  it('disables Resume with a reason when the project folder is gone', () => {
    const onResume = vi.fn();
    render(
      <SessionRow
        session={session({ cwd_exists: false, cwd: '/w/deleted' })}
        expanded
        onToggle={vi.fn()}
        onResume={onResume}
      />,
    );
    const btn = screen.getByRole('button', { name: /^Resume / }) as HTMLButtonElement;
    expect(btn.disabled).toBe(true);
    expect(btn.title).toContain('/w/deleted');
    fireEvent.click(btn);
    expect(onResume).not.toHaveBeenCalled();
  });

  // Resume splits into the two surfaces it can launch into. Both are real
  // buttons in the DOM at all times — the collapsed "Resume" face is decoration
  // — so a keyboard user reaches the same two choices a hover reveals.
  it('offers a terminal and a VS Code option when the folder exists', () => {
    const onResume = vi.fn();
    render(
      <SessionRow
        session={session()}
        expanded
        vsCodeAvailable
        onToggle={vi.fn()}
        onResume={onResume}
      />,
    );
    const term = screen.getByRole('button', { name: /Resume .* in a terminal/ }) as HTMLButtonElement;
    const code = screen.getByRole('button', {
      name: /Resume .* in a VS Code tab/,
    }) as HTMLButtonElement;
    expect(term.disabled).toBe(false);
    expect(code.disabled).toBe(false);

    fireEvent.click(term);
    expect(onResume).toHaveBeenCalledWith('029a3e04-fa36', 'terminal');
    fireEvent.click(code);
    expect(onResume).toHaveBeenCalledWith('029a3e04-fa36', 'vs_code_tab');
  });

  // Clicking through to a failure inside VS Code is worse than saying up front
  // that the editor half is missing.
  it('disables the VS Code option with a reason when VS Code is unavailable', () => {
    render(
      <SessionRow
        session={session()}
        expanded
        vsCodeAvailable={false}
        onToggle={vi.fn()}
        onResume={vi.fn()}
      />,
    );
    const code = screen.getByRole('button', {
      name: /Resume .* in a VS Code tab/,
    }) as HTMLButtonElement;
    expect(code.disabled).toBe(true);
    expect(code.title).toMatch(/code.*PATH|extension/i);
    expect(
      (screen.getByRole('button', { name: /in a terminal/ }) as HTMLButtonElement).disabled,
    ).toBe(false);
  });

  // Re-applying bypassPermissions is what the user wants, but it removes a
  // safety check, so it must be stated next to the button that applies it.
  it('states the permission mode the resume will re-apply', () => {
    render(
      <SessionRow
        session={session({ permission_mode: 'bypassPermissions' })}
        expanded
        onToggle={vi.fn()}
        onResume={vi.fn()}
      />,
    );
    expect(screen.getByText('bypass permissions')).toBeTruthy();
  });

  it('says nothing about permissions when the session recorded no mode', () => {
    render(<SessionRow session={session()} expanded onToggle={vi.fn()} onResume={vi.fn()} />);
    expect(screen.queryByText(/permission/i)).toBeNull();
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

  // The window comes from the model's registry entry, not from how full the
  // session got. Sonnet 5 / Opus 4.7+ / Fable 5 carry `native_1m: true` and
  // run at 1M with no `[1m]` suffix, so a quiet session on one of them is
  // still a 1M session.
  it('reports the model’s real window even when the session stayed small', () => {
    render(
      <SessionRow
        session={session({ model: 'claude-fable-5', peak_context_tokens: 155_837 })}
        expanded
        onToggle={vi.fn()}
        onResume={vi.fn()}
      />,
    );
    expect(screen.getByText(/155\.8K of 1M/)).toBeTruthy();
  });

  it('reports 200K for an older model in the same family', () => {
    render(
      <SessionRow
        session={session({ model: 'claude-sonnet-4-6', peak_context_tokens: 57_000 })}
        expanded
        onToggle={vi.fn()}
        onResume={vi.fn()}
      />,
    );
    expect(screen.getByText(/57K of 200K/)).toBeTruthy();
  });

  // The fixture's model is glm-5.2 — a third-party window is provider-set and
  // absent from the transcript, so no denominator is invented.
  it('shows a bare token count for a third-party model', () => {
    render(
      <SessionRow session={session()} expanded onToggle={vi.fn()} onResume={vi.fn()} />,
    );
    expect(screen.getByText(/^410K$/)).toBeTruthy();
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

  it('shows the session’s lifetime tokens and cost on the collapsed row', () => {
    render(
      <SessionRow session={session()} expanded={false} onToggle={vi.fn()} onResume={vi.fn()} />,
    );
    expect(screen.getByText('12.3K')).toBeTruthy();
    expect(screen.getByText('$1.23')).toBeTruthy();
  });

  // A transcript with no ingested usage must render blank, not "0 $0.00" —
  // a zero here would read as a measurement rather than an absence.
  it('renders nothing for a session with no ingested usage', () => {
    render(
      <SessionRow
        session={session({ total_tokens: 0, total_cost_usd: 0 })}
        expanded={false}
        onToggle={vi.fn()}
        onResume={vi.fn()}
      />,
    );
    expect(screen.queryByText('$0.00')).toBeNull();
    expect(screen.queryByText(/^0$/)).toBeNull();
  });

  it('calls onResume with the session id and the chosen surface', () => {
    const onResume = vi.fn();
    render(
      <SessionRow
        session={session()}
        expanded
        vsCodeAvailable
        onToggle={vi.fn()}
        onResume={onResume}
      />,
    );
    fireEvent.click(screen.getByRole('button', { name: /in a terminal/ }));
    expect(onResume).toHaveBeenCalledWith(session().session_id, 'terminal');
  });
});
