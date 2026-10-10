import {
  HELP_LINK_TEXT,
  helpLinkName,
  type ContextualHelp,
} from "./contextualLinks";
import { useHelp } from "./useHelp";

/** "How does this work?", opening Help at `to` (#423). Take `to` from
 * `CONTEXTUAL_HELP`, never a page written in place. The words on screen never
 * change; the accessible name adds the target's topic, so a screen reader
 * says which of several links on a screen this one is (#460). */
export function ContextualHelpLink({ to }: { to: ContextualHelp }) {
  const { openHelp } = useHelp();
  return (
    <button
      type="button"
      className="help-link"
      aria-label={helpLinkName(to)}
      onClick={() => openHelp(to.page, to.heading)}
    >
      {HELP_LINK_TEXT}
    </button>
  );
}
