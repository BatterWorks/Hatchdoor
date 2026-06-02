import "@testing-library/jest-dom/vitest";
import { afterEach } from "vitest";

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

afterEach(() => {
  MockEventSource.instances.length = 0;
});

declare global {
  interface Window {
    __hatchdoorEventSources: MockEventSource[];
  }
}
