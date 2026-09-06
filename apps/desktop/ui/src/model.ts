export type NoteStatus = 'draft' | 'final';

export type ModelState = 'not_downloaded' | 'downloading' | 'ready' | 'error' | 'canceled';

export interface ModelStatus {
  model_id: string;
  state: ModelState;
  downloaded_bytes: number;
  total_bytes: number;
  percent: number;
  current_asset?: string | null;
  install_path?: string | null;
  error?: string | null;
}

export function modelStatusLabel(status: ModelStatus | null): string {
  if (!status) return 'Checking model';
  switch (status.state) {
    case 'ready': return 'Ready to record';
    case 'downloading': return `Downloading ${status.percent}%`;
    case 'error': return 'Download needs attention';
    case 'canceled': return 'Download canceled';
    default: return 'Model not downloaded';
  }
}

export interface TranscriptSegment {
  id: string;
  sequence: number;
  startSeconds: number;
  endSeconds: number;
  text: string;
  provisional: boolean;
  speaker?: string;
}

export interface Citation {
  startSeconds: number;
  endSeconds: number;
}

export interface NoteItem {
  text: string;
  citation?: Citation;
}

export interface MeetingNotes {
  status: NoteStatus;
  summary: string;
  decisions: NoteItem[];
  actionItems: NoteItem[];
  openQuestions: NoteItem[];
}

export interface MeetingExport {
  title: string;
  transcript: TranscriptSegment[];
  notes: MeetingNotes;
}

export interface TranscriptEventPayload {
  id?: string;
  sequence?: number;
  window_sequence?: number;
  startSeconds?: number;
  endSeconds?: number;
  start_seconds?: number;
  end_seconds?: number;
  startMicros?: number;
  endMicros?: number;
  start_micros?: number;
  end_micros?: number;
  text?: string;
  provisional?: boolean;
  speaker?: string | null;
}

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === 'object' && value !== null;

const finiteNumber = (value: unknown): number | undefined =>
  typeof value === 'number' && Number.isFinite(value) ? value : undefined;

const secondsFrom = (value: unknown, micros: unknown): number => {
  const seconds = finiteNumber(value);
  if (seconds !== undefined) return Math.max(0, seconds);
  const microseconds = finiteNumber(micros);
  return microseconds === undefined ? 0 : Math.max(0, microseconds / 1_000_000);
};

const textFrom = (value: unknown): string =>
  typeof value === 'string' ? value.trim() : '';

/** Convert either snake_case Tauri payloads or UI-shaped payloads to one safe shape. */
export function normalizeTranscriptPayload(
  payload: unknown,
  fallbackSequence = 0,
): TranscriptSegment {
  const value = isRecord(payload) ? payload : {};
  const sequenceValue = finiteNumber(value.sequence ?? value.window_sequence);
  const sequence = sequenceValue === undefined ? fallbackSequence : Math.max(0, sequenceValue);
  const text = textFrom(value.text ?? value.transcript);
  const speaker = textFrom(value.speaker);
  const id = textFrom(value.id) || `segment-${sequence}`;

  return {
    id,
    sequence,
    startSeconds: secondsFrom(value.startSeconds ?? value.start_seconds, value.startMicros ?? value.start_micros),
    endSeconds: secondsFrom(value.endSeconds ?? value.end_seconds, value.endMicros ?? value.end_micros),
    text,
    provisional: value.provisional === true,
    ...(speaker ? { speaker } : {}),
  };
}

const shouldUseIncoming = (current: TranscriptSegment, incoming: TranscriptSegment): boolean => {
  if (current.provisional && !incoming.provisional) return true;
  if (!current.provisional && incoming.provisional) return false;
  if (!incoming.text && current.text) return false;
  return true;
};

/** Merge sequence-keyed event deltas while protecting authoritative text from stale drafts. */
export function mergeTranscriptSegments(
  current: readonly TranscriptSegment[],
  incoming: readonly TranscriptSegment[],
): TranscriptSegment[] {
  const bySequence = new Map<number, TranscriptSegment>();
  for (const segment of current) bySequence.set(segment.sequence, segment);
  for (const segment of incoming) {
    const existing = bySequence.get(segment.sequence);
    if (!existing || shouldUseIncoming(existing, segment)) bySequence.set(segment.sequence, segment);
  }
  return [...bySequence.values()].sort((left, right) => left.sequence - right.sequence);
}

export function formatDuration(totalSeconds: number): string {
  const seconds = Math.max(0, Math.floor(Number.isFinite(totalSeconds) ? totalSeconds : 0));
  const hours = Math.floor(seconds / 3_600);
  const minutes = Math.floor((seconds % 3_600) / 60);
  const remainder = seconds % 60;
  if (hours > 0) {
    return [hours, minutes, remainder].map((part) => String(part).padStart(2, '0')).join(':');
  }
  return `${String(minutes).padStart(2, '0')}:${String(remainder).padStart(2, '0')}`;
}

export function formatCitation(citation: Citation): string {
  return `${formatDuration(citation.startSeconds)}–${formatDuration(citation.endSeconds)}`;
}

const citationFor = (segment: TranscriptSegment): Citation => ({
  startSeconds: segment.startSeconds,
  endSeconds: Math.max(segment.startSeconds, segment.endSeconds),
});

const noteItem = (segment: TranscriptSegment): NoteItem => ({
  text: segment.text,
  citation: citationFor(segment),
});

const meaningfulSegments = (transcript: readonly TranscriptSegment[]): TranscriptSegment[] =>
  transcript.filter((segment) => segment.text.trim().length > 0);

const looksLikeDecision = (text: string): boolean =>
  /\b(decid(?:e|ed|ing)|agreed|approved|ship(?:s|ped)?|launch(?:es|ed)?|will use|go with)\b/i.test(text);

const looksLikeAction = (text: string): boolean =>
  /\b(will|todo|to-do|owner|follow up|send|share|schedule|assigned)\b/i.test(text);

const looksLikeQuestion = (text: string): boolean =>
  /\?|\b(open question|question|unclear|need to confirm)\b/i.test(text);

const firstMatching = (
  transcript: readonly TranscriptSegment[],
  predicate: (text: string) => boolean,
): NoteItem[] => {
  const matches = transcript.filter((segment) => predicate(segment.text));
  return matches.map(noteItem);
};

const summaryFor = (transcript: readonly TranscriptSegment[]): string => {
  const text = meaningfulSegments(transcript).slice(0, 2).map((segment) => segment.text).join(' ');
  return text || 'No transcript content yet.';
};

function buildNotes(transcript: readonly TranscriptSegment[], status: NoteStatus): MeetingNotes {
  const content = meaningfulSegments(transcript);
  const decisions = firstMatching(content, looksLikeDecision);
  const actionItems = firstMatching(content, looksLikeAction);
  const openQuestions = firstMatching(content, looksLikeQuestion);

  return {
    status,
    summary: summaryFor(content),
    decisions: decisions.length > 0 ? decisions : content.slice(0, 1).map(noteItem),
    actionItems,
    openQuestions,
  };
}

export function buildDraftNotes(transcript: readonly TranscriptSegment[]): MeetingNotes {
  return buildNotes(transcript, 'draft');
}

export function buildFinalNotes(transcript: readonly TranscriptSegment[]): MeetingNotes {
  return buildNotes(transcript, 'final');
}

const markdownItem = (item: NoteItem): string =>
  `- ${item.text}${item.citation ? ` [${formatCitation(item.citation)}]` : ''}`;

export function serializeMeetingExport(
  meeting: MeetingExport,
  format: 'markdown' | 'json',
): string {
  if (format === 'json') return JSON.stringify(meeting, null, 2);

  const section = (heading: string, items: readonly NoteItem[]): string =>
    `## ${heading}\n${items.length > 0 ? items.map(markdownItem).join('\n') : '- None recorded.'}`;
  const transcript = meeting.transcript.length > 0
    ? meeting.transcript
      .map((segment) => `- ${formatCitation({ startSeconds: segment.startSeconds, endSeconds: segment.endSeconds })} ${segment.speaker ? `${segment.speaker}: ` : ''}${segment.text}`)
      .join('\n')
    : '- No transcript captured.';

  return [
    `# ${meeting.title}`,
    '',
    '## Summary',
    meeting.notes.summary,
    '',
    section('Decisions', meeting.notes.decisions),
    '',
    section('Action items', meeting.notes.actionItems),
    '',
    section('Open questions', meeting.notes.openQuestions),
    '',
    '## Transcript',
    transcript,
    '',
  ].join('\n');
}
