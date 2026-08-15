/** `timestamp` is an ISO string (Git/index status) or an epoch-ms number
 * (held-draft `savedAt`, #151) — both resolve to the same relative-age
 * ladder rather than duplicating it per caller. */
export function formatWhen(
  timestamp: string | number | null | undefined,
): string | null {
  if (!timestamp) return null;
  const then = typeof timestamp === "number" ? timestamp : Date.parse(timestamp);
  if (!Number.isFinite(then)) return null;
  const minutes = Math.round((Date.now() - then) / 60_000);
  if (minutes < 1) return "just now";
  if (minutes === 1) return "1 minute ago";
  if (minutes < 60) return `${minutes} minutes ago`;
  const hours = Math.round(minutes / 60);
  if (hours === 1) return "1 hour ago";
  if (hours < 24) return `${hours} hours ago`;
  const days = Math.round(hours / 24);
  return days === 1 ? "1 day ago" : `${days} days ago`;
}
