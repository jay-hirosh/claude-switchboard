import { motion } from 'framer-motion';
import { useAppStore } from '../lib/store';
import { IconButton } from '../components/ui/IconButton';
import { AccountRow } from './AccountRow';
import { AddAccountChooser } from './AddAccountChooser';
import { SwapConfirmCard } from './SwapConfirmCard';
import { ModalShell } from '../components/modals/ModalShell';
import { IconRefresh } from '../lib/icons';
import { useAccountManagement } from './useAccountManagement';

export function AccountsSidebar() {
  const thresholds = useAppStore(
    (s) => (s.settings?.thresholds ?? [75, 90]) as [number, number],
  );

  const {
    accounts,
    orgGroups,
    currentActive,
    bestAccountUuid,
    pending,
    swappingSlot,
    confirmError,
    refreshing,
    reauthSlot,
    chooserOpen,
    requestSwap,
    confirmSwap,
    cancelSwap,
    handleReauth,
    handleRemove,
    handleRefreshAll,
    openChooser,
    closeChooser,
  } = useAccountManagement();

  return (
    <aside
      className="
        flex flex-col h-full
        w-[300px] shrink-0
        border-r border-[var(--color-rule)]
        bg-[var(--color-bg-surface)]
      "
    >
      <div className="flex items-center justify-between px-[var(--space-lg)] pt-[var(--space-md)] pb-[var(--space-sm)] shrink-0">
        <span className="text-[length:var(--text-label)] uppercase tracking-[var(--tracking-label)] text-[color:var(--color-text-secondary)] font-[var(--weight-semibold)]">
          Accounts
        </span>
        <IconButton label="Refresh all" onClick={handleRefreshAll}>
          <motion.span
            animate={refreshing ? { rotate: 360 } : { rotate: 0 }}
            transition={
              refreshing
                ? { duration: 0.7, ease: 'linear', repeat: Infinity }
                : { duration: 0.2 }
            }
            style={{ display: 'inline-flex' }}
          >
            <IconRefresh size={13} />
          </motion.span>
        </IconButton>
      </div>

      <div className="flex-1 overflow-y-auto">
        {accounts.length === 0 && (
          <div className="px-[var(--space-lg)] py-[var(--space-md)] text-[length:var(--text-micro)] text-[color:var(--color-text-muted)]">
            No accounts managed yet.
          </div>
        )}
        {accounts.map((a) => {
          const groupHead = a.org_uuid ? orgGroups.get(a.org_uuid) : undefined;
          const shareHint = groupHead && groupHead.slot !== a.slot ? groupHead.email : null;
          return (
            <AccountRow
              key={a.slot}
              entry={a}
              thresholds={thresholds}
              shareHint={shareHint}
              isBest={a.account_uuid === bestAccountUuid}
              onSwap={() => requestSwap(a)}
              swapBusy={swappingSlot !== null}
              swapping={swappingSlot === a.slot}
              onReauth={() => handleReauth(a)}
              reauthBusy={reauthSlot === a.slot}
              onRemove={() => handleRemove(a)}
            />
          );
        })}
      </div>

      <div className="shrink-0 px-[var(--space-lg)] py-[var(--space-md)] border-t border-[var(--color-rule)]">
        <button
          type="button"
          onClick={openChooser}
          className="text-[length:var(--text-label)] text-[color:var(--color-accent)] hover:underline"
        >
          + Add account
        </button>
      </div>

      {chooserOpen && (
        <ModalShell id="add-account-chooser" onDismiss={closeChooser} title="Add account">
          <AddAccountChooser presentation="modal" onClose={closeChooser} />
        </ModalShell>
      )}

      {pending && (
        <ModalShell id="swap-confirm" onDismiss={cancelSwap} title="Confirm switch">
          <SwapConfirmCard
            presentation="modal"
            current={currentActive}
            target={pending.target}
            running={pending.running}
            busy={swappingSlot !== null}
            errorMessage={confirmError}
            onConfirm={confirmSwap}
            onCancel={cancelSwap}
          />
        </ModalShell>
      )}
    </aside>
  );
}
