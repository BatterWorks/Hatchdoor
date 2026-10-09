import "@testing-library/jest-dom/vitest";
import { cleanup } from "@testing-library/react";
import { afterEach, vi } from "vitest";

// Test files share one jsdom and one module cache per worker (`isolate: false`
// in vite.config.ts), and Vitest runs this file again before each of them.
// Put back what a fresh environment would give the next file.
//
// Source modules an earlier file imported stay evaluated otherwise, so their
// state carries over and this file's `vi.mock` never reaches them. Packages
// in node_modules are loaded by Node, not Vitest, and are not reset: the
// imports above are the same instances the test file gets.
vi.resetModules();

// Stylesheets an earlier file imported. `css: true` injects them into the
// head, and they can hide elements from role queries here. Modules this
// file imports inject their own again.
for (const style of document.head.querySelectorAll("style[data-vite-dev-id]")) {
  style.remove();
}

for (const element of [document.documentElement, document.body]) {
  for (const { name } of [...element.attributes]) {
    element.removeAttribute(name);
  }
}
document.body.replaceChildren();
localStorage.clear();

type EventSourceListener = (event: MessageEvent<string>) => void;

class MockEventSource {
  static instances: MockEventSource[] = [];

  readonly url: string;
  private listeners = new Map<string, EventSourceListener[]>();

  constructor(url: string | URL) {
    this.url = String(url);
    MockEventSource.instances.push(this);
  }

  addEventListener(type: string, listener: EventSourceListener) {
    const listeners = this.listeners.get(type) ?? [];
    listeners.push(listener);
    this.listeners.set(type, listeners);
  }

  removeEventListener(type: string, listener: EventSourceListener) {
    const listeners = this.listeners.get(type) ?? [];
    this.listeners.set(
      type,
      listeners.filter((item) => item !== listener),
    );
  }

  close() {}

  emit(type: string, data: string) {
    const event = new MessageEvent(type, { data });
    for (const listener of this.listeners.get(type) ?? []) {
      listener(event);
    }
  }
}

Object.defineProperty(window, "matchMedia", {
  writable: true,
  value: (query: string) => ({
    matches: false,
    media: query,
    onchange: null,
    addEventListener: () => {},
    removeEventListener: () => {},
    dispatchEvent: () => false,
  }),
});

Object.defineProperty(window, "EventSource", {
  writable: true,
  value: MockEventSource,
});

Object.defineProperty(globalThis, "EventSource", {
  writable: true,
  value: MockEventSource,
});

Object.defineProperty(window, "__hatchdoorEventSources", {
  writable: true,
  value: MockEventSource.instances,
});

// Testing Library only unmounts automatically when `afterEach` is a global,
// and this config leaves Vitest's globals off. Without it a test sees the
// DOM of whichever test ran before it, which `--sequence.shuffle` exposes.
// This `cleanup` sees the test file's renders only because the package is
// not inlined (see `server.deps.inline`), so both share one instance.
afterEach(cleanup);

afterEach(() => {
  MockEventSource.instances.length = 0;
});

declare global {
  interface Window {
    __hatchdoorEventSources: MockEventSource[];
  }
}

// CodeMirror measures text through Range client rects, which jsdom does not
// implement; the live editor mounts in tests only with these in place.
if (typeof Range !== "undefined") {
  const emptyRects = () =>
    Object.assign([], { item: () => null }) as unknown as DOMRectList;
  if (!Range.prototype.getClientRects) {
    Range.prototype.getClientRects = emptyRects;
  }
  if (!Range.prototype.getBoundingClientRect) {
    Range.prototype.getBoundingClientRect = () =>
      ({
        x: 0,
        y: 0,
        width: 0,
        height: 0,
        top: 0,
        left: 0,
        right: 0,
        bottom: 0,
        toJSON: () => ({}),
      }) as DOMRect;
  }
}
