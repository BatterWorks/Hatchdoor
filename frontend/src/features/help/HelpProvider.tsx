import { useCallback, useMemo, useRef, useState, type ReactNode } from "react";

import { HELP_HOME, type HelpLocation } from "./helpPages";
import { HelpPanel } from "./HelpPanel";
import { HelpContext } from "./useHelp";

type Visit = HelpLocation & {
  /** Bumped on every visit, so opening the same heading twice scrolls twice. */
  visit: number;
};

/**
 * Owns the Help panel (#417): whether it is open, which page it shows, and
 * the pages behind it for Back. Mounted above the token prompt and the
 * workspace, so either can open it, signed in or not.
 */
export function HelpProvider({
  children,
  demoMode = false,
  signedOut = false,
  onOpenSetupChecklist,
}: {
  children: ReactNode;
  demoMode?: boolean;
  /** The token prompt is up; Help then sits above it. */
  signedOut?: boolean;
  /** Reopens the first-run checklist (#419). Home lists a "Setup checklist"
   * entry only when this is given. */
  onOpenSetupChecklist?: () => void;
}) {
  const [isOpen, setIsOpen] = useState(false);
  const [fullWidth, setFullWidth] = useState(false);
  const [current, setCurrent] = useState<Visit>({ page: HELP_HOME, visit: 0 });
  const [history, setHistory] = useState<Visit[]>([]);
  const returnFocus = useRef<HTMLElement | null>(null);
  const visitCount = useRef(0);

  const stampVisit = useCallback(
    (location: HelpLocation): Visit => ({
      ...location,
      visit: (visitCount.current += 1),
    }),
    [],
  );

  const openHelp = useCallback(
    (page?: string, heading?: string) => {
      if (!isOpen) {
        const active = document.activeElement;
        returnFocus.current = active instanceof HTMLElement ? active : null;
      }
      setIsOpen(true);
      setHistory([]);
      setCurrent(stampVisit({ page: page ?? HELP_HOME, heading }));
    },
    [isOpen, stampVisit],
  );

  const closeHelp = useCallback(() => {
    setIsOpen(false);
    const target = returnFocus.current;
    returnFocus.current = null;
    if (target?.isConnected) {
      target.focus();
    }
  }, []);

  const navigate = useCallback(
    (location: HelpLocation) => {
      setHistory((previous) => [...previous, current]);
      setCurrent(stampVisit(location));
    },
    [current, stampVisit],
  );

  const back = useCallback(() => {
    const last = history.at(-1);
    if (!last) {
      return;
    }
    setHistory(history.slice(0, -1));
    setCurrent(stampVisit({ page: last.page, heading: last.heading }));
  }, [history, stampVisit]);

  const helpApi = useMemo(
    () => ({ openHelp, closeHelp, isOpen }),
    [openHelp, closeHelp, isOpen],
  );

  return (
    <HelpContext.Provider value={helpApi}>
      {children}
      {isOpen ? (
        <HelpPanel
          location={current}
          canGoBack={history.length > 0}
          fullWidth={fullWidth}
          demoMode={demoMode}
          aboveDialogs={signedOut}
          onOpenSetupChecklist={
            onOpenSetupChecklist
              ? () => {
                  closeHelp();
                  onOpenSetupChecklist();
                }
              : undefined
          }
          onNavigate={navigate}
          onBack={back}
          onHome={() => navigate({ page: HELP_HOME })}
          onToggleFullWidth={() => setFullWidth((value) => !value)}
          onClose={closeHelp}
        />
      ) : null}
    </HelpContext.Provider>
  );
}
