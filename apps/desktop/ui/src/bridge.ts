import { formatCitation, formatDuration, type TranscriptSegment } from './model';

export const UI_EVENTS = {
  closeRequested: 'session-close-requested',
  transcript: 'transcript',
  transcriptUpdate: 'transcript-update',
  liveNotes: 'live-notes',
  finalNotes: 'final-notes',
  session: 'session-state',
  health: 'capture-health',
  finalization: 'finalization-progress',
  error: 'worker-error',
} as const;

export type UiEventName = (typeof UI_EVENTS)[keyof typeof UI_EVENTS];
export type Unlisten = () => void;
export type TauriInvoke = (command: string, args?: Record<string, unknown>) => Promise<unknown>;
export type TauriListen = (event: string, handler: (event: { payload: unknown }) => void) => Promise<Unlisten>;

export interface TauriApi {
  core?: { invoke?: TauriInvoke };
  event?: { listen?: TauriListen };
}

export interface WindowWithTauri extends Window {
  __TAURI__?: TauriApi;
}

declare global {
  interface Window {
    __TAURI__?: TauriApi;
  }
}

export function tauriApi(windowObject: Window = window): TauriApi | undefined {
  return (windowObject as WindowWithTauri).__TAURI__;
}

export function bridgeInvoke(windowObject: Window = window): TauriInvoke | undefined {
  return tauriApi(windowObject)?.core?.invoke;
}

export function bridgeListen(windowObject: Window = window): TauriListen | undefined {
  return tauriApi(windowObject)?.event?.listen;
}

export function invokeOrDemo(
  command: string,
  args?: Record<string, unknown>,
  windowObject: Window = window,
): Promise<unknown> {
  const invoke = bridgeInvoke(windowObject);
  return invoke ? invoke(command, args) : Promise.resolve(undefined);
}

export async function listenToEvents(
  names: readonly UiEventName[],
  handler: (name: UiEventName, payload: unknown) => void,
  windowObject: Window = window,
): Promise<Unlisten> {
  const listen = bridgeListen(windowObject);
  if (!listen) return () => undefined;

  const unlisteners = await Promise.all(names.map((name) =>
    listen(name, (event) => handler(name, event.payload)),
  ));
  return () => unlisteners.forEach((unlisten) => unlisten());
}

export async function listenToEvent(
  name: UiEventName,
  handler: (payload: unknown) => void,
  windowObject: Window = window,
): Promise<Unlisten> {
  return listenToEvents([name], (_eventName, payload) => handler(payload), windowObject);
}

export function formatTimer(seconds: number): string {
  return formatDuration(seconds);
}

export function segmentLabel(segment: TranscriptSegment): string {
  return `${formatCitation({ startSeconds: segment.startSeconds, endSeconds: segment.endSeconds })}${segment.speaker ? ` · ${segment.speaker}` : ''}`;
}
