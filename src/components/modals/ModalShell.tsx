import { useEffect, useLayoutEffect, useRef } from 'react';
import { useAppStore } from '../../lib/store';
import { IconButton } from '../ui/IconButton';
import { X } from '../../lib/icons';

interface Props {
  id: string;
  onDismiss: () => void;
  title?: string;
  size?: 'sm' | 'md' | 'lg';
  /** When false, ESC, backdrop click, and the title-bar X are all disabled.
   *  Used by forced-decision dialogs like WarmupConsentModal. Default true. */
  dismissable?: boolean;
  children: React.ReactNode;
}

const SIZE_CLASSES: Record<NonNullable<Props['size']>, string> = {
  sm: 'max-w-[320px]',
  md: 'max-w-[480px]',
  lg: 'max-w-[640px]',
};

export function ModalShell({
  id,
  onDismiss,
  title,
  size = 'md',
  dismissable = true,
  children,
}: Props) {
  const pushModal = useAppStore((s) => s.pushModal);
  const popModal = useAppStore((s) => s.popModal);
  const isTopmost = useAppStore((s) => s.isTopmost);
  const stackDepth = useAppStore((s) => s.modalStack.indexOf(id));

  const cardRef = useRef<HTMLDivElement>(null);

  // useLayoutEffect commits before paint so the z-index calculation reflects
  // stack position on the first painted frame. useEffect would leave one
  // pre-commit frame at the wrong z when two modals mount in the same tick.
  useLayoutEffect(() => {
    pushModal(id);
    return () => popModal(id);
  }, [id, pushModal, popModal]);

  // Focus trap: capture previously-focused element, focus the card on mount,
  // restore on unmount. Spec §4.2 requires a focus trap so keyboard users
  // can't Tab off the modal into underlying content.
  useLayoutEffect(() => {
    const prev = document.activeElement as HTMLElement | null;
    cardRef.current?.focus();
    return () => prev?.focus();
  }, []);

  useEffect(() => {
    if (!dismissable) return;
    function handleKey(e: KeyboardEvent) {
      if (e.key === 'Escape' && isTopmost(id)) {
        onDismiss();
      }
    }
    window.addEventListener('keydown', handleKey);
    return () => window.removeEventListener('keydown', handleKey);
  }, [id, isTopmost, onDismiss, dismissable]);

  // Tab/Shift+Tab wrap. Only the topmost modal traps focus so a stacked
  // modal-on-top owns the trap.
  useEffect(() => {
    function handleTab(e: KeyboardEvent) {
      if (e.key !== 'Tab') return;
      if (!isTopmost(id)) return;
      const card = cardRef.current;
      if (!card) return;
      const focusables = card.querySelectorAll<HTMLElement>(
        'button, [href], input, select, textarea, [tabindex]:not([tabindex="-1"])',
      );
      if (focusables.length === 0) return;
      const first = focusables[0];
      const last = focusables[focusables.length - 1];
      const active = document.activeElement as HTMLElement | null;
      if (e.shiftKey && active === first) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && active === last) {
        e.preventDefault();
        first.focus();
      }
    }
    window.addEventListener('keydown', handleTab);
    return () => window.removeEventListener('keydown', handleTab);
  }, [id, isTopmost]);

  const z = 50 + 10 * Math.max(0, stackDepth);
  const titleId = title ? `${id}-title` : undefined;

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-labelledby={titleId}
      data-testid="modal-backdrop"
      onClick={() => {
        if (dismissable && isTopmost(id)) onDismiss();
      }}
      className="fixed inset-0 flex items-center justify-center p-4"
      style={{
        zIndex: z,
        background: 'var(--color-overlay)',
        // `position: fixed` resolves against the viewport — the full,
        // sharp-cornered window rect — and #root's `overflow: hidden` does
        // NOT clip it, because a fixed element escapes ancestor overflow.
        // So the scrim paints into the four corners that #root's radius
        // leaves bare, and the window grows dark wedges around its rounded
        // shell for as long as a modal is open. Repeat the window radius.
        borderRadius: 'var(--window-radius, var(--radius-lg))',
      }}
    >
      <div
        ref={cardRef}
        tabIndex={-1}
        onClick={(e) => e.stopPropagation()}
        className={`
          w-full ${SIZE_CLASSES[size]} max-h-full overflow-y-auto
          rounded-[var(--radius-lg)]
          border
          shadow-[0_12px_36px_oklch(0%_0_0_/_0.4)]
          outline-none
        `}
        style={{
          background: 'var(--color-bg-elevated)',
          borderColor: 'var(--color-border)',
        }}
      >
        {title && (
          <div className="flex items-center justify-between px-[var(--space-md)] py-[var(--space-sm)] border-b border-[var(--color-rule)]">
            <span
              id={titleId}
              className="text-[length:var(--text-label)] font-[var(--weight-semibold)] uppercase tracking-[var(--tracking-label)] text-[color:var(--color-text-secondary)]"
            >
              {title}
            </span>
            {dismissable && (
              <IconButton label="Close" onClick={onDismiss}>
                <X size={13} />
              </IconButton>
            )}
          </div>
        )}
        {children}
      </div>
    </div>
  );
}
