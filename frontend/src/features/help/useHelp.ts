import { createContext, useContext } from "react";

export type HelpApi = {
  /** Open Help at a page of the manual, scrolled to `heading` (a heading
   * anchor such as `where-do-i-find-my-token`). No page opens Home. */
  openHelp: (page?: string, heading?: string) => void;
  closeHelp: () => void;
  isOpen: boolean;
};

const NO_HELP: HelpApi = {
  openHelp: () => {},
  closeHelp: () => {},
  isOpen: false,
};

export const HelpContext = createContext<HelpApi>(NO_HELP);

/** The open-Help entry point. Outside a `HelpProvider` it does nothing. */
export function useHelp(): HelpApi {
  return useContext(HelpContext);
}
