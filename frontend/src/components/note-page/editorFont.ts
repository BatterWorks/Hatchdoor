/**
 * The canvas `font` string for an element, as CSS actually computed it.
 *
 * `getComputedStyle().font` is the obvious source and silently serializes to
 * the empty string whenever a longhand cannot be folded back into the
 * shorthand. The heading fonts do exactly that, through their
 * `font-variation-settings`, and an empty string assigned to a canvas context
 * is *ignored* rather than rejected: the measurement then silently runs in
 * whatever font the shared context was last set to, so a heading was measured
 * in the body font and hung far too little. Composing the longhands is the only
 * reading that is always true.
 */
export function resolveFont(style: {
  font: string;
  fontStyle: string;
  fontWeight: string;
  fontSize: string;
  lineHeight: string;
  fontFamily: string;
}): string {
  if (style.font) {
    return style.font;
  }
  const lineHeight =
    style.lineHeight && style.lineHeight !== "normal"
      ? `/${style.lineHeight}`
      : "";
  return `${style.fontStyle} ${style.fontWeight} ${style.fontSize}${lineHeight} ${style.fontFamily}`.trim();
}
