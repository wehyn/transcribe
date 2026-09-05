import test from 'node:test';
import assert from 'node:assert/strict';
import {
  buildDraftNotes,
  buildFinalNotes,
  formatDuration,
  mergeTranscriptSegments,
  normalizeTranscriptPayload,
  serializeMeetingExport,
} from '../src/model.ts';

const segment = (sequence: number, text: string, provisional = false) => ({
  id: `segment-${sequence}`,
  sequence,
  startSeconds: sequence * 7,
  endSeconds: sequence * 7 + 7,
  text,
  provisional,
  speaker: sequence === 0 ? 'Mia' : undefined,
});

test('formats elapsed time with stable hour, minute, and second padding', () => {
  assert.equal(formatDuration(0), '00:00');
  assert.equal(formatDuration(65), '01:05');
  assert.equal(formatDuration(3723), '01:02:03');
});

test('normalizes Tauri transcript payloads and trims untrusted text', () => {
  const result = normalizeTranscriptPayload(
    {
      sequence: 3,
      start_micros: 1_500_000,
      end_micros: 3_000_000,
      text: '  Hello from the worker  ',
      provisional: true,
      speaker: 'Speaker 1',
    },
    0,
  );

  assert.deepEqual(result, {
    id: 'segment-3',
    sequence: 3,
    startSeconds: 1.5,
    endSeconds: 3,
    text: 'Hello from the worker',
    provisional: true,
    speaker: 'Speaker 1',
  });
});

test('merges ordered transcript updates without letting provisional text replace final text', () => {
  const initial = [segment(2, 'draft ending', true), segment(0, 'Opening')];
  const merged = mergeTranscriptSegments(initial, [segment(1, 'Middle'), segment(2, 'final ending')]);
  const stale = mergeTranscriptSegments(merged, [segment(2, 'stale draft', true)]);

  assert.deepEqual(merged.map(({ sequence, text, provisional }) => ({ sequence, text, provisional })), [
    { sequence: 0, text: 'Opening', provisional: false },
    { sequence: 1, text: 'Middle', provisional: false },
    { sequence: 2, text: 'final ending', provisional: false },
  ]);
  assert.equal(stale[2].text, 'final ending');
});

test('builds draft and final notes with citations from transcript segments', () => {
  const transcript = [
    segment(0, 'The beta ships on Friday.'),
    segment(1, 'Mia will send the customer update.'),
    segment(2, 'Open question: should pricing be in onboarding?'),
  ];

  const draft = buildDraftNotes(transcript);
  const final = buildFinalNotes(transcript);

  assert.equal(draft.status, 'draft');
  assert.equal(final.status, 'final');
  assert.match(final.summary, /beta ships on Friday/);
  assert.ok(final.decisions[0].citation);
  assert.ok(final.actionItems[0].citation);
  assert.ok(final.openQuestions[0].citation);
});

test('serializes final review exports as markdown or JSON', () => {
  const transcript = [segment(0, 'Ship the beta on Friday.')];
  const notes = buildFinalNotes(transcript);
  const markdown = serializeMeetingExport({ title: 'Weekly sync', transcript, notes }, 'markdown');
  const json = serializeMeetingExport({ title: 'Weekly sync', transcript, notes }, 'json');

  assert.match(markdown, /# Weekly sync/);
  assert.match(markdown, /00:00–00:07/);
  assert.deepEqual(JSON.parse(json), {
    title: 'Weekly sync',
    transcript,
    notes,
  });
});
