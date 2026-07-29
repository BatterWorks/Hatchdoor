import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { parseFrontmatter } from "../../lib/markdown";
import { NoteProperties } from "./sections";

afterEach(() => cleanup());

function Harness({
  initialContent,
  onContentChange,
  editable = true,
}: {
  initialContent: string;
  onContentChange?: (next: string) => void;
  editable?: boolean;
}) {
  const [content, setContent] = useState(initialContent);
  return (
    <NoteProperties
      properties={parseFrontmatter(content).properties}
      content={content}
      editable={editable}
      collapsed={false}
      onToggleCollapsed={() => {}}
      onTagSelect={() => {}}
      onChange={(next) => {
        setContent(next);
        onContentChange?.(next);
      }}
    />
  );
}

const NOTE = `---
title: Home
tags:
  - work
  - urgent
---
# Body
`;

describe("editable frontmatter properties", () => {
  it("shows a property value as text until it is clicked", () => {
    render(<Harness initialContent={NOTE} />);

    expect(screen.queryByRole("textbox")).toBeNull();
    expect(screen.getByText("Home")).toBeInTheDocument();
  });

  it("opens an input on the clicked value", () => {
    render(<Harness initialContent={NOTE} />);

    fireEvent.click(screen.getByText("Home"));

    expect(screen.getByRole("textbox")).toHaveValue("Home");
  });

  it("writes the edited value back into the frontmatter only", () => {
    const onChange = vi.fn();
    render(<Harness initialContent={NOTE} onContentChange={onChange} />);

    fireEvent.click(screen.getByText("Home"));
    fireEvent.change(screen.getByRole("textbox"), {
      target: { value: "Homepage" },
    });
    fireEvent.blur(screen.getByRole("textbox"));

    expect(onChange).toHaveBeenCalledWith(
      "---\ntitle: Homepage\ntags:\n  - work\n  - urgent\n---\n# Body\n",
    );
  });

  it("edits a list property as comma separated values", () => {
    const onChange = vi.fn();
    render(<Harness initialContent={NOTE} onContentChange={onChange} />);

    fireEvent.click(screen.getByText("#work"));
    fireEvent.change(screen.getByRole("textbox"), {
      target: { value: "work, later" },
    });
    fireEvent.blur(screen.getByRole("textbox"));

    expect(onChange).toHaveBeenCalledWith(
      "---\ntitle: Home\ntags:\n  - work\n  - later\n---\n# Body\n",
    );
  });

  it("does not open an input in a read-only vault", () => {
    render(<Harness initialContent={NOTE} editable={false} />);

    fireEvent.click(screen.getByText("Home"));

    expect(screen.queryByRole("textbox")).toBeNull();
  });
});
