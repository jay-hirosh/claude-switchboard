import { useLayoutEffect, useRef, useState } from 'react';
import { motion } from 'framer-motion';
import { IconButton } from '../components/ui/IconButton';
import { SessionsTab } from './SessionsTab';
import { ModelsTab } from './ModelsTab';
import { TrendsTab } from './TrendsTab';
import { ProjectsTab } from './ProjectsTab';
import { HeatmapTab } from './HeatmapTab';
import { CacheTab } from './CacheTab';
import { useAppStore } from '../lib/store';
import { ipc } from '../lib/ipc';
import { tabSlide } from '../lib/motion';
import { IconRefresh, IconCollapse, IconSettings, X } from '../lib/icons';
import { handleDragStart, closeWindow } from '../lib/window-chrome';
import { AccountsSidebar } from '../accounts/AccountsSidebar';
import { SettingsModal } from '../components/modals/SettingsModal';
import { ProvidersTab } from '../providers/ProvidersTab';
import { SessionsBrowserTab } from '../sessions/SessionsBrowserTab';

const TAB_CONFIG = [
  { id: 'browse', label: 'Sessions' },
  { id: 'cost', label: 'Cost' },
  { id: 'models', label: 'Models' },
  { id: 'trends', label: 'Trends' },
  { id: 'projects', label: 'Projects' },
  { id: 'heatmap', label: 'Heatmap' },
  { id: 'cache', label: 'Cache' },
  { id: 'providers', label: 'Providers' },
] as const;

const TAB_COMPONENTS: Record<string, React.FC> = {
  browse: SessionsBrowserTab,
  cost: SessionsTab,
  models: ModelsTab,
  trends: TrendsTab,
  projects: ProjectsTab,
  heatmap: HeatmapTab,
  cache: CacheTab,
  providers: ProvidersTab,
};

export function ExpandedReport() {
  const [activeTab, setActiveTab] = useState<string>('browse');
  const [refreshing, setRefreshing] = useState(false);
  const [tabKey, setTabKey] = useState(0);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const prevTabRef = useRef<string>('browse');
  const stale = useAppStore((s) => s.stale);
  const toggleViewMode = useAppStore((s) => s.toggleViewMode);

  const TabComponent = TAB_COMPONENTS[activeTab] ?? SessionsTab;

  const tabIds = TAB_CONFIG.map((t) => t.id) as string[];
  const prevIdx = tabIds.indexOf(prevTabRef.current);
  const currIdx = tabIds.indexOf(activeTab);
  const slideDir = currIdx >= prevIdx ? 1 : -1;
  prevTabRef.current = activeTab;

  async function handleRefresh() {
    if (refreshing) return;
    setRefreshing(true);
    try {
      await ipc.forceRefresh('active');
      setTabKey((k) => k + 1);
    } finally {
      setTimeout(() => setRefreshing(false), 420);
    }
  }

  return (
    <>
      <div
        className="flex h-full overflow-hidden"
        style={{
          width: '100%',
          minHeight: 'var(--report-min-height)',
          background: 'var(--color-bg-base)',
        }}
      >
        <AccountsSidebar />
        <div className="flex flex-1 flex-col overflow-hidden min-w-0">
          {/* Header — generous padding, brand-warm tinted strip with hairline below */}
          <header
            onPointerDown={handleDragStart}
            className="
              relative flex items-center justify-between gap-[var(--space-md)]
              px-[var(--space-2xl)] pt-[var(--space-xl)] pb-[var(--space-lg)]
              shrink-0 cursor-default select-none
            "
          >
            <div className="flex items-center gap-[var(--space-xs)] pointer-events-none">
              <span className="text-[length:var(--text-label)] font-[var(--weight-semibold)] text-[color:var(--color-text-secondary)] tracking-[var(--tracking-label)] uppercase">
                Claude
              </span>
              <span className="text-[length:var(--text-label)] tracking-[var(--tracking-label)] uppercase text-[color:var(--color-text-muted)]">
                · {stale ? 'Stale' : 'Live'} · last 30 days
              </span>
            </div>
            <div className="flex items-center gap-[2px]">
              <IconButton label="Refresh" onClick={handleRefresh}>
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
              <IconButton label="Settings" onClick={() => setSettingsOpen(true)}>
                <IconSettings size={13} />
              </IconButton>
              <IconButton label="Collapse details" onClick={toggleViewMode}>
                <IconCollapse size={13} />
              </IconButton>
              <IconButton label="Close" onClick={closeWindow}>
                <X size={13} />
              </IconButton>
            </div>
          </header>

          {/* Tab bar — text-only, with a single sliding underline indicator */}
          <TabBar
            activeId={activeTab}
            onSelect={setActiveTab}
            tabs={TAB_CONFIG.map((t) => ({ id: t.id, label: t.label }))}
          />

          {/* Tab content */}
          <div className="flex-1 overflow-y-auto px-[var(--space-2xl)] pb-[var(--space-2xl)] pt-[var(--space-lg)]">
            <motion.div
              key={`${activeTab}-${tabKey}`}
              variants={tabSlide}
              initial="enter"
              animate="center"
              exit="exit"
              custom={slideDir}
            >
              <TabComponent />
            </motion.div>
          </div>
        </div>
      </div>
      {settingsOpen && <SettingsModal onDismiss={() => setSettingsOpen(false)} />}
    </>
  );
}

/* ───────────────────────── TabBar ─────────────────────────
 *
 * Horizontal text nav with one moving underline that slides between tabs
 * (Apple's site nav, Linear's view switcher). Layout-tracked via refs so the
 * underline matches the actual rendered button width — no manual measuring.
 */
function TabBar({
  activeId,
  onSelect,
  tabs,
}: {
  activeId: string;
  onSelect: (id: string) => void;
  tabs: { id: string; label: string }[];
}) {
  const containerRef = useRef<HTMLDivElement>(null);
  const buttonRefs = useRef<Record<string, HTMLButtonElement | null>>({});
  const [indicator, setIndicator] = useState({ x: 0, w: 0 });

  useLayoutEffect(() => {
    const container = containerRef.current;
    const btn = buttonRefs.current[activeId];
    if (!container || !btn) return;
    const measure = () => {
      const cRect = container.getBoundingClientRect();
      // Use a Range over the button's text node — that hugs the actual
      // glyphs and ignores any width the box adds beyond them (UA padding,
      // border, font side-bearings, letter-spacing trail). Falls back to
      // the button rect if the range is empty (no text yet).
      const range = document.createRange();
      range.selectNodeContents(btn);
      const tRect = range.getBoundingClientRect();
      const rect = tRect.width > 0 ? tRect : btn.getBoundingClientRect();
      setIndicator({ x: rect.left - cRect.left, w: rect.width });
    };
    measure();
    // Window resize and font load can change the active button's metrics
    // after first paint. Without re-measuring, the indicator drifts off the
    // text — most visibly when the system font lazy-resolves and glyph
    // widths shift between fallback and SF Pro.
    const ro = new ResizeObserver(measure);
    ro.observe(btn);
    ro.observe(container);
    return () => ro.disconnect();
  }, [activeId, tabs.length]);

  return (
    <div
      ref={containerRef}
      role="tablist"
      className="
        relative flex items-center gap-[var(--space-xl)]
        px-[var(--space-2xl)]
        border-b border-[var(--color-rule)]
        shrink-0
      "
    >
      {tabs.map((tab) => {
        const active = activeId === tab.id;
        return (
          <button
            key={tab.id}
            ref={(el) => {
              buttonRefs.current[tab.id] = el;
            }}
            role="tab"
            aria-selected={active}
            type="button"
            onClick={() => onSelect(tab.id)}
            className={[
              'relative inline-flex items-center',
              'h-[44px]',
              // p-0 border-0 are explicit because WKWebView (Tauri on macOS)
              // gives <button> a default 2px–6px UA padding that Tailwind
              // preflight does not fully reset, inflating the measured
              // bounding rect past the visible text and dragging the sliding
              // underline indicator with it.
              'p-0 border-0 bg-transparent',
              'text-[length:var(--text-label)] font-[var(--weight-medium)]',
              'tracking-[var(--tracking-label)] uppercase',
              'transition-colors duration-[var(--duration-fast)] ease-[var(--ease-out)]',
              'cursor-default',
              active
                ? 'text-[color:var(--color-text)]'
                : 'text-[color:var(--color-text-muted)] hover:text-[color:var(--color-text-secondary)]',
              'focus-visible:outline-2 focus-visible:outline-[var(--color-border-focus)] focus-visible:outline-offset-2 rounded',
            ].join(' ')}
          >
            {tab.label}
          </button>
        );
      })}
      {/* Sliding underline */}
      <motion.span
        aria-hidden
        className="absolute bottom-0 left-0 h-[2px] rounded-full"
        style={{ background: 'var(--color-accent)' }}
        initial={false}
        animate={{ x: indicator.x, width: indicator.w }}
        transition={{ type: 'spring', stiffness: 380, damping: 32, mass: 0.7 }}
      />
    </div>
  );
}
