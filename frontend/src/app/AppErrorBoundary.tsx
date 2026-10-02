import { Component, type ErrorInfo, type ReactNode } from "react";

import { StateBlock } from "../components/ui";

/**
 * The top-level error boundary (#339). A render that throws with nothing above
 * it leaves React rendering nothing at all: a blank page with no message and
 * no way to tell what happened. This turns that into a sentence and a reload.
 * It is a last resort, not a recovery path: storage reads on the boot path are
 * already guarded (`lib/storage.ts`'s `safeGetItem`), so this should only ever
 * catch a fault nobody has seen yet.
 */
export class AppErrorBoundary extends Component<
  { children: ReactNode },
  { failed: boolean }
> {
  state = { failed: false };

  static getDerivedStateFromError(): { failed: boolean } {
    return { failed: true };
  }

  componentDidCatch(error: unknown, info: ErrorInfo): void {
    console.error("Hatchdoor failed to render", error, info.componentStack);
  }

  render() {
    if (this.state.failed) {
      return (
        <div className="startup-shell">
          <StateBlock
            tone="error"
            title="Something Went Wrong"
            description="Hatchdoor could not display this page. Reload to try again; your notes are unaffected."
            actionLabel="Reload"
            onAction={() => window.location.reload()}
          />
        </div>
      );
    }
    return this.props.children;
  }
}
