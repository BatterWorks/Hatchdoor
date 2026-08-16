import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { StateBlock } from "./ui";

afterEach(cleanup);

describe("StateBlock", () => {
  it("renders the plain empty shell by default", () => {
    const { container } = render(
      <StateBlock title="Nothing Here" description="Nothing to show yet." />,
    );

    const block = container.querySelector(".state-block");
    expect(block).not.toBeNull();
    expect(block).not.toHaveClass("error");
  });

  it("renders the documented error variant when tone is error (#141)", () => {
    const { container } = render(
      <StateBlock tone="error" title="Note Unavailable" description="boom" />,
    );

    expect(container.querySelector(".state-block.error")).not.toBeNull();
    expect(screen.getByText("Note Unavailable")).toBeInTheDocument();
  });
});
