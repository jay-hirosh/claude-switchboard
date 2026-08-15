import type { AccountListEntry } from '../../lib/generated/bindings';
import { colorForAccount, labelForAccount } from '../../report/accountDisplay';

interface Props {
  accountUuid: string | null;
  accounts: AccountListEntry[];
  className?: string;
}

/**
 * A colored dot + short label identifying which account a row's data
 * belongs to. Renders alongside `ModelBadge` rather than replacing it —
 * model and account are independent facets of the same row. Color is
 * per-account (dynamic, not one of `Badge`'s fixed variants), so this
 * renders its own pill rather than composing `Badge`.
 */
export function AccountBadge({ accountUuid, accounts, className = '' }: Props) {
  const color = colorForAccount(accountUuid, accounts);
  const label = labelForAccount(accountUuid, accounts);
  return (
    <span
      className={[
        'inline-flex items-center gap-[var(--space-2xs)]',
        'px-[7px] py-[2px]',
        'rounded-[var(--radius-pill)]',
        'text-[length:var(--text-micro)] font-[var(--weight-medium)]',
        'select-none',
        'bg-[var(--color-track)]',
        className,
      ].join(' ')}
    >
      <span aria-hidden className="w-[6px] h-[6px] rounded-full shrink-0" style={{ background: color }} />
      <span style={{ color }}>{label}</span>
    </span>
  );
}
