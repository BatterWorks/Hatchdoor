import { useEffect, useState } from "react";

import { CONTEXTUAL_HELP, useHelp } from "../help";
import {
  fetchUpdateStatus,
  markDismissed,
  readDismissed,
  releaseToShow,
  type AvailableRelease,
} from "./updateBanner";

/**
 * The update banner (#425): when the opt-in update check found a newer
 * release, one line in the shell's notice strip with a "What's new" link to
 * the GitHub release page and a "How to upgrade" link into Help. Dismissed
 * per version, per browser. The shell mounts it signed in and never in demo
 * mode; it shows nothing when the check is off or the request fails.
 */
export function UpdateBanner() {
  const [release, setRelease] = useState<AvailableRelease | null>(null);
  const { openHelp } = useHelp();

  useEffect(() => {
    const controller = new AbortController();
    void fetchUpdateStatus(controller.signal).then((status) => {
      if (!controller.signal.aborted) {
        setRelease(releaseToShow(status, readDismissed()));
      }
    });
    return () => controller.abort();
  }, []);

  if (!release) {
    return null;
  }
  return (
    <div className="write-notice" role="status">
      <div className="write-notice-messages">
        <span>
          Hatchdoor {release.version} is available.{" "}
          <a
            className="help-link"
            href={release.release_url}
            target="_blank"
            rel="noopener noreferrer"
          >
            What's new
          </a>{" "}
          <button
            type="button"
            className="help-link"
            onClick={() => openHelp(CONTEXTUAL_HELP.upgrade.page)}
          >
            How to upgrade
          </button>
        </span>
      </div>
      <button
        type="button"
        className="write-notice-dismiss"
        aria-label="Dismiss update notice"
        onClick={() => {
          markDismissed(release.version);
          setRelease(null);
        }}
      >
        ×
      </button>
    </div>
  );
}
