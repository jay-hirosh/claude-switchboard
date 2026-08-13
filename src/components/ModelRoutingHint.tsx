import { useState } from 'react';
import { Banner } from './ui/Banner';
import type { ModelRoutingHint as ModelRoutingHintData } from '../lib/modelRoutingHint';
import { dismissRoutingHint, isRoutingHintDismissed } from '../lib/routingHintDismissal';

interface Props {
  accountId: string;
  hint: ModelRoutingHintData;
}

const LABEL: Record<'opus' | 'sonnet', string> = { opus: 'Opus', sonnet: 'Sonnet' };

export function ModelRoutingHint({ accountId, hint }: Props) {
  const [dismissed, setDismissed] = useState(() =>
    isRoutingHintDismissed(accountId, hint.resetsAt),
  );
  if (dismissed) return null;

  const quieter = hint.busier === 'opus' ? 'sonnet' : 'opus';

  return (
    <Banner
      variant="warning"
      onDismiss={() => {
        dismissRoutingHint(accountId, hint.resetsAt);
        setDismissed(true);
      }}
    >
      {LABEL[hint.busier]} 7D at {Math.round(hint.busierPct)}%, {LABEL[quieter]} at{' '}
      {Math.round(hint.quieterPct)}% — consider <code>/model {quieter}</code>
    </Banner>
  );
}
