import type { HelpLocation } from "./helpPages";
import { useHelp } from "./useHelp";

/** "How does this work?", opening Help at `to` (#423). Take `to` from
 * `CONTEXTUAL_HELP`, never a page written in place. */
export function ContextualHelpLink({ to }: { to: HelpLocation }) {
  const { openHelp } = useHelp();
  return (
    <button
      type="button"
      className="help-link"
      onClick={() => openHelp(to.page, to.heading)}
    >
      How does this work?
    </button>
  );
}
