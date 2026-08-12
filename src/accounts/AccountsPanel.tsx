import { motion } from 'framer-motion';
import { useAppStore } from '../lib/store';
import { IconButton } from '../components/ui/IconButton';
import { AccountRow } from './AccountRow';
import { AddAccountChooser } from './AddAccountChooser';
import { SwapConfirmCard } from './SwapConfirmCard';
import { ModalShell } from '../components/modals/ModalShell';
import { IconRefresh, X } from '../lib/icons';
import { closeWindow } from '../lib/window-chrome';
import { useAccountManagement } from './useAccountManagement';

interface Props {
  onBack: () => void;
}

export function AccountsPanel({ onBack }: Props) {
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
    <div className="flex h-full w-full flex-col">
      <div className="flex items-center justify-between px-[var(--popover-pad)] pt-[var(--space-md)] pb-[var(--space-sm)]">
        <button
          type="button"
          onClick={onBack}
          className="text-[length:var(--text-label)] text-[color:var(--color-text-secondary)] hover:text-[color:var(--color-text)]"
        >
          ← Back
        </button>
        <span className="text-[length:var(--text-label)] uppercase tracking-[var(--tracking-label)] text-[color:var(--color-text-secondary)]">
          Accounts
        </span>
        <div className="flex items-center gap-[2px]">
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
          <IconButton label="Close" onClick={closeWindow}>
            <X size={13} />
          </IconButton>
        </div>
      </div>

      <div className="flex-1 overflow-y-auto">
        {accounts.length === 0 && (
          <div className="px-[var(--popover-pad)] py-[var(--space-md)] text-[color:var(--color-text-muted)]">
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

        <div className="px-[var(--popover-pad)] py-[var(--space-md)]">
          <button
            type="button"
            onClick={openChooser}
            className="text-[length:var(--text-label)] text-[color:var(--color-accent)] hover:underline"
          >
            + Add account
          </button>
        </div>
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
    </div>
  );
}
