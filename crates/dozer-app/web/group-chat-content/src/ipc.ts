import type { OutEvent } from './types.ts';

declare global {
  interface Window {
    ipc?: { postMessage(body: string): void };
    __dozer?: { dispatch(json: string): void };
  }
}

export function send(event: OutEvent): void {
  window.ipc?.postMessage(JSON.stringify(event));
}
