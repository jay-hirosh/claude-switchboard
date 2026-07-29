import { useEffect, useState } from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import { CompactPopover } from './popover/CompactPopover';
import { ExpandedReport } from './report/ExpandedReport';
import { AuthPanel } from './settings/AuthPanel';
import { useAppStore } from './lib/store';
import { useThemeStore, resolveTheme, type ResolvedTheme } from './lib/theme';
import { attachUpdateListeners } from './lib/updateEvents';
import './styles/globals.css';
import './styles/tokens.css';

export function App() {
  const init = useAppStore((s) => s.init);
  const accounts = useAppStore((s) => s.accounts);
  const viewMode = useAppStore((s) => s.viewMode);
  const [initialized, setInitialized] = useState(false);

  useEffect(() => {
    init().finally(() => setInitialized(true));
  }, [init]);

  useEffect(() => {
    let teardown: (() => void) | null = null;
    attachUpdateListeners().then((unlisten) => { teardown = unlisten; });
    return () => { teardown?.(); };
  }, []);

  const themePreference = useThemeStore((s) => s.themePreference);

  useEffect(() => {
    const mql = window.matchMedia('(prefers-color-scheme: dark)');
    const apply = () => {
      const resolved: ResolvedTheme = resolveTheme(themePreference, mql.matches);
      document.body.dataset.theme = resolved;
    };
    apply();
    if (themePreference === 'auto') {
      mql.addEventListener('change', apply);
      return () => mql.removeEventListener('change', apply);
    }
  }, [themePreference]);

  useEffect(() => {
    document.body.dataset.viewMode = viewMode;
    // The Windows corner radius lives in globals.css, not here: it has to be
    // 0 so the webview paints every pixel of the client area, and the OS does
    // the rounding. Setting it from JS as well left two sources of truth.
    if (navigator.userAgent.includes('Windows')) {
      document.body.dataset.os = 'windows';
    }
    return () => { delete document.body.dataset.viewMode; };
  }, [viewMode]);

  if (!initialized) {
    return (
      <div className="flex h-full w-full items-center justify-center p-6">
        <span className="text-[color:var(--color-text-muted)]">Loading…</span>
      </div>
    );
  }

  // No managed accounts → always route to AuthPanel. This covers both the
  // first-run case (no live CC creds either) and the fresh-CC-login case
  // (live creds exist but haven't been imported yet) — in the latter the
  // "Use upstream's current login" tile in AuthPanel imports the live
  // account in one click. Without this, the popover would render
  // LoadingShell forever because state.snapshot() returns None until
  // active_slot resolves to a managed slot.
  if (accounts.length === 0) {
    return <AuthPanel />;
  }

  const isWin = navigator.userAgent.includes('Windows');

  const content = (
    <AnimatePresence mode="wait" initial={false}>
      {viewMode === 'expanded' ? (
        <motion.div
          key="expanded"
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={{ duration: 0.14, ease: [0.16, 1, 0.3, 1] }}
          style={{ height: '100%' }}
        >
          <ExpandedReport />
        </motion.div>
      ) : (
        <motion.div
          key="compact"
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={{ duration: 0.14, ease: [0.16, 1, 0.3, 1] }}
          style={{ height: '100%' }}
        >
          <CompactPopover />
        </motion.div>
      )}
    </AnimatePresence>
  );

  return (
    <>
      {isWin ? (
        <motion.div
          initial={false}
          animate={{
            width: viewMode === 'expanded' ? 960 : 360,
            height: viewMode === 'expanded' ? 640 : 380,
          }}
          transition={{ duration: 0.28, ease: [0.16, 1, 0.3, 1] }}
          style={{ overflow: 'hidden', margin: 'auto' }}
          className="win-animated-container"
        >
          {content}
        </motion.div>
      ) : (
        content
      )}
    </>
  );
}
