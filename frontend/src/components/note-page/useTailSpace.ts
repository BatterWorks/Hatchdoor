import { useCallback, useEffect, useState } from "react";

/**
 * Whether this note is long enough to deserve the trailing scroll space, and
 * the ref to put on the element that carries it.
 *
 * The space exists so a heading near the end of a note can still be scrolled
 * to the top of the pane. A short note has no such heading and no reason to
 * scroll at all, so carrying a screenful of emptiness below it just gives the
 * reader somewhere pointless to go.
 *
 * The measurement subtracts whatever trailing space is currently applied
 * before comparing, which is what keeps it from chasing its own tail: adding
 * the space cannot make a note look long enough to need the space.
 *
 * A callback ref rather than a `useRef` object, because the article mounts a
 * render later than the note's content arrives — keying the effect on the
 * content alone measured while there was still nothing in the DOM to measure.
 */
export function useTailSpace(): {
  needsTail: boolean;
  contentRef: (node: HTMLElement | null) => void;
} {
  const [content, setContent] = useState<HTMLElement | null>(null);
  const [needsTail, setNeedsTail] = useState(false);

  const contentRef = useCallback((node: HTMLElement | null) => {
    setContent(node);
  }, []);

  useEffect(() => {
    if (!content) {
      return;
    }
    const scroller = content.closest<HTMLElement>(".note-pane");
    if (!scroller) {
      return;
    }

    const measure = () => {
      const tail = parseFloat(getComputedStyle(content).paddingBottom) || 0;
      setNeedsTail(content.scrollHeight - tail > scroller.clientHeight);
    };

    measure();

    if (typeof ResizeObserver === "undefined") {
      return;
    }
    // The article grows after mount — wikilinks resolve, images and KaTeX
    // land — so one measurement at mount is never the final word.
    const observer = new ResizeObserver(measure);
    observer.observe(content);
    observer.observe(scroller);
    return () => observer.disconnect();
  }, [content]);

  return { needsTail, contentRef };
}
