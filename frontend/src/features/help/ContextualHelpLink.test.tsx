import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { ContextualHelpLink } from "./ContextualHelpLink";
import { CONTEXTUAL_HELP } from "./contextualLinks";
import { HelpContext } from "./useHelp";

afterEach(cleanup);

describe("ContextualHelpLink (#460)", () => {
  it.each(Object.entries(CONTEXTUAL_HELP))(
    "%s reads How does this work? and announces its topic after it",
    (_key, target) => {
      render(<ContextualHelpLink to={target} />);
      const link = screen.getByRole("button", {
        name: `How does this work? ${target.topic}`,
      });
      expect(link).toHaveTextContent(/^How does this work\?$/);
    },
  );

  it("tells two links on one screen apart by name and opens each at its own target", () => {
    const openHelp = vi.fn();
    render(
      <HelpContext.Provider
        value={{ openHelp, closeHelp: () => {}, isOpen: false }}
      >
        <ContextualHelpLink to={CONTEXTUAL_HELP.noVaults} />
        <ContextualHelpLink to={CONTEXTUAL_HELP.updateCheck} />
      </HelpContext.Provider>,
    );

    fireEvent.click(
      screen.getByRole("button", {
        name: "How does this work? Hearing about new releases",
      }),
    );
    expect(openHelp).toHaveBeenLastCalledWith(
      CONTEXTUAL_HELP.updateCheck.page,
      CONTEXTUAL_HELP.updateCheck.heading,
    );

    fireEvent.click(
      screen.getByRole("button", {
        name: "How does this work? Adding your notes",
      }),
    );
    expect(openHelp).toHaveBeenLastCalledWith(
      CONTEXTUAL_HELP.noVaults.page,
      undefined,
    );
  });
});
