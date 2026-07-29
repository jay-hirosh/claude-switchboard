import { type SelectHTMLAttributes, forwardRef, useId } from 'react';

interface SelectOption {
  value: string;
  label: string;
}

interface SelectProps extends Omit<SelectHTMLAttributes<HTMLSelectElement>, 'size'> {
  label: string;
  options: SelectOption[];
}

export const Select = forwardRef<HTMLSelectElement, SelectProps>(
  ({ label, options, className = '', ...props }, ref) => {
    const id = useId();

    return (
      <div className={['flex flex-col gap-[var(--space-2xs)]', className].join(' ')}>
        <label
          htmlFor={id}
          className="text-[length:var(--text-label)] font-[var(--weight-medium)] text-[color:var(--color-text-secondary)]"
        >
          {label}
        </label>
        <div className="relative">
          <select
            ref={ref}
            id={id}
            className={[
              'w-full appearance-none',
              'bg-[var(--color-field)] border border-[var(--color-border)]',
              'rounded-[var(--radius-sm)]',
              'px-[var(--space-sm)] py-[var(--space-xs)]',
              'text-[length:var(--text-body)] text-[color:var(--color-text)]',
              'outline-none',
              'transition-[border-color] duration-[var(--duration-fast)]',
              'hover:border-[var(--color-border-hover)]',
              'focus:border-[var(--color-border-focus)]',
              'cursor-pointer',
            ].join(' ')}
            {...props}
          >
            {options.map((opt) => (
              <option key={opt.value} value={opt.value}>
                {opt.label}
              </option>
            ))}
          </select>
          <div className="pointer-events-none absolute right-[var(--space-sm)] top-1/2 -translate-y-1/2 text-[color:var(--color-text-muted)]">
            <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
              <path d="m6 9 6 6 6-6" />
            </svg>
          </div>
        </div>
      </div>
    );
  },
);

Select.displayName = 'Select';
