/**
 * Shared class strings for form fields inside a modal.
 *
 * These lived as private constants in ProviderForm, so every other form built
 * its inputs by hand and drifted — the resume picker ended up with a raw
 * `<select>` carrying the browser's own chevron and none of the token set.
 * One definition, imported everywhere, is what "two screens must feel like the
 * same app" needs.
 */

export const inputClass = [
  'w-full rounded-[var(--radius-sm)]',
  'border border-[var(--color-border)] bg-[var(--color-field)]',
  'px-[var(--space-sm)] py-[var(--space-xs)]',
  'text-[length:var(--text-body)] text-[color:var(--color-text)]',
  'placeholder:text-[color:var(--color-text-muted)]',
  'transition-[border-color] duration-[var(--duration-fast)]',
  'focus:border-[var(--color-border-focus)] focus:outline-none',
].join(' ');

/** `inputClass` for a `<select>`. `appearance-none` drops the platform
 *  chevron so `SelectChevron` can draw one in our own palette — the native
 *  double-arrow is the single most out-of-place thing in a themed dialog. */
export const selectClass = [
  inputClass,
  'appearance-none cursor-pointer pr-[28px]',
  'hover:border-[var(--color-border-hover)]',
].join(' ');

export const labelClass = [
  'text-[length:var(--text-micro)] font-[var(--weight-medium)]',
  'uppercase tracking-[var(--tracking-label)]',
  'text-[color:var(--color-text-muted)]',
].join(' ');

export const hintClass =
  'text-[length:var(--text-micro)] leading-[var(--leading-body)] text-[color:var(--color-text-muted)]';
