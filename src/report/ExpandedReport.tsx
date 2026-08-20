import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import type { MouseEvent as ReactMouseEvent } from 'react';
import { motion } from 'framer-motion';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { IconButton } from '../components/ui/IconButton';
import { SessionsTab, WINDOW_DAYS } from './SessionsTab';
import { DashboardTab } from './DashboardTab';
import { ModelsTab } from './ModelsTab';
import { TrendsTab } from './TrendsTab';
import { RepoTab } from './RepoTab';
import { HeatmapTab } from './HeatmapTab';
import { CacheTab } from './CacheTab';
import { LimitHitsTab } from './LimitHitsTab';
import { useAppStore } from '../lib/store';
import { ipc } from '../lib/ipc';
import { tabSlide } from '../lib/motion';
import { IconRefresh, IconCollapse, IconSettings, IconFullscreen, IconExitFullscreen, X } from '../lib/icons';
import { handleDragStart, closeWindow } from '../lib/window-chrome';
import { isFullscreenShortcut } from '../lib/fullscreenShortcut';
import { AccountsSidebar } from '../accounts/AccountsSidebar';
import { SettingsModal } from '../components/modals/SettingsModal';
import { ProvidersTab } from '../providers/ProvidersTab';
import { SessionsBrowserTab } from '../sessions/SessionsBrowserTab';

const TAB_CONFIG = [
  { id: 'today', label: 'Dashboard' },
  { id: 'repo', label: 'Repository' },
  { id: 'browse', label: 'Sessions' },
  { id: 'cost', label: 'Cost' },
  { id: 'models', label: 'Models' },
  { id: 'trends', label: 'Trends' },
  { id: 'limits', label: 'Limit hits' },
  { id: 'heatmap', label: 'Heatmap' },
  { id: 'cache', label: 'Cache' },
  { id: 'providers', label: 'Providers' },
] as const;

/** How far back each tab actually queries. The header used to hardcode
 *  "last 30 days" for every tab, which was wrong for Cost (7) and Heatmap
 *  (180) — a Cost total that omitted three weeks of spend was labelled as if
 *  it included them. Keep these in step with the `ipc.get*(n)` call in each
 *  tab; `undefined` means the tab has no time window of its own. */
const TAB_WINDOW_DAYS: Record<string, number | undefined> = {
  today: undefined,
  repo: undefined,
  browse: undefined,
  cost: WINDOW_DAYS,
  models: 30,
  // Trends now has two sub-tabs on different windows (Usage's own 7d/30d
  // toggle, Daily pattern's 7-90d lookback) — no single number is right for
  // the header caption, so each sub-panel labels its own range inline.
  trends: undefined,
  limits: 30,
  heatmap: 180,
  cache: 30,
  providers: undefined,
};

const TAB_COMPONENTS: Record<string, React.FC> = {
  today: DashboardTab,
  repo: RepoTab,
  browse: SessionsBrowserTab,
  cost: SessionsTab,
  models: ModelsTab,
  trends: TrendsTab,
  limits: LimitHitsTab,
  heatmap: HeatmapTab,
  cache: CacheTab,
  providers: ProvidersTab,
};

export function ExpandedReport() {
  const [activeTab, setActiveTab] = useState<string>('today');
  const [refreshing, setRefreshing] = useState(false);
  const [tabKey, setTabKey] = useState(0);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const prevTabRef = useRef<string>('today');
  const stale = useAppStore((s) => s.stale);
  const toggleViewMode = useAppStore((s) => s.toggleViewMode);
  const isFullscreen = useAppStore((s) => s.isFullscreen);
  const toggleFullscreen = useAppStore((s) => s.toggleFullscreen);
  const setFullscreen = useAppStore((s) => s.setFullscreen);

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

  // Collapsing or closing while still in native fullscreen would strand the
  // (much smaller) resulting window inside an empty fullscreen Space —
  // always exit fullscreen first.
  async function handleCollapse() {
    if (isFullscreen) await toggleFullscreen();
    toggleViewMode();
  }

  async function handleClose() {
    if (isFullscreen) await toggleFullscreen();
    closeWindow();
  }

  function handleHeaderDoubleClick(e: ReactMouseEvent<HTMLElement>) {
    const target = e.target as HTMLElement;
    if (target.closest('button, input, a, select, textarea')) return;
    toggleFullscreen();
  }

  useEffect(() => {
    function onKeyDown(e: KeyboardEvent) {
      if (!isFullscreenShortcut(e)) return;
      e.preventDefault();
      toggleFullscreen();
    }
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [toggleFullscreen]);

  useEffect(() => {
    // The OS can exit fullscreen on its own (Mission Control, etc.) without
    // going through toggleFullscreen() — resync on every resize, which fires
    // whenever fullscreen is entered or exited either way.
    let unlisten: (() => void) | undefined;
    (async () => {
      try {
        const win = getCurrentWindow();
        unlisten = await win.onResized(async () => {
          try {
            setFullscreen(await win.isFullscreen());
          } catch {
            // no-op
          }
        });
      } catch {
        // outside Tauri
      }
    })();
    return () => unlisten?.();
  }, [setFullscreen]);

  return (
    <>
      <div
        className="flex h-full overflow-hidden print:h-auto print:overflow-visible"
        style={{
          width: '100%',
          minHeight: 'var(--report-min-height)',
          background: 'var(--color-bg-base)',
        }}
      >
        <div className="print:hidden contents">
          <AccountsSidebar />
        </div>
        <div className="flex flex-1 flex-col overflow-hidden min-w-0 print:overflow-visible">
          {/* Header — generous padding, brand-warm tinted strip with hairline below */}
          <header
            onPointerDown={handleDragStart}
            onDoubleClick={handleHeaderDoubleClick}
            className="
              relative flex items-center justify-between gap-[var(--space-md)]
              px-[var(--space-2xl)] pt-[var(--space-xl)] pb-[var(--space-lg)]
              shrink-0 cursor-default select-none
              print:hidden
            "
          >
            <div className="flex items-center gap-[var(--space-xs)] pointer-events-none">
              <span className="text-[length:var(--text-label)] font-[var(--weight-semibold)] text-[color:var(--color-text-secondary)] tracking-[var(--tracking-label)] uppercase">
                Claude
              </span>
              <span className="text-[length:var(--text-label)] tracking-[var(--tracking-label)] uppercase text-[color:var(--color-text-muted)]">
                · {stale ? 'Stale' : 'Live'}
                {TAB_WINDOW_DAYS[activeTab]
                  ? ` · last ${TAB_WINDOW_DAYS[activeTab]} days`
                  : activeTab === 'today'
                    ? ' · Dashboard'
                    : ''}
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
              <IconButton
                label={isFullscreen ? 'Exit fullscreen' : 'Enter fullscreen'}
                onClick={toggleFullscreen}
              >
                {isFullscreen ? <IconExitFullscreen size={13} /> : <IconFullscreen size={13} />}
              </IconButton>
              <IconButton label="Collapse details" onClick={handleCollapse}>
                <IconCollapse size={13} />
              </IconButton>
              <IconButton label="Close" onClick={handleClose}>
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

          {/* Tab content — capped and centered so fullscreen on a large or
             ultrawide display doesn't stretch charts/tables edge to edge;
             a no-op at the normal 960px window width. */}
          <div className="flex-1 overflow-y-auto px-[var(--space-2xl)] pb-[var(--space-2xl)] pt-[var(--space-lg)] print:overflow-visible print:h-auto print:px-0 print:pt-0">
            <motion.div
              key={`${activeTab}-${tabKey}`}
              variants={tabSlide}
              initial="enter"
              animate="center"
              exit="exit"
              custom={slideDir}
              style={{ maxWidth: 'var(--report-content-max-width)', marginInline: 'auto' }}
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
        print:hidden
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
