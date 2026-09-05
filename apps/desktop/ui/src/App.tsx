import { useEffect, useMemo, useState } from 'react';
import './styles.css';
import {
  buildDraftNotes,
  buildFinalNotes,
  formatCitation,
  mergeTranscriptSegments,
  normalizeTranscriptPayload,
  modelStatusLabel,
  serializeMeetingExport,
  type MeetingNotes,
  type NoteItem,
  type ModelStatus,
  type TranscriptSegment,
} from './model';
import {
  invokeOrDemo,
  listenToEvents,
  listenToEvent,
  UI_EVENTS,
  type UiEventName,
} from './bridge';

type Language = 'english' | 'filipino' | 'taglish';
type SessionState = 'setup' | 'recording' | 'paused' | 'stopped' | 'processing' | 'ready' | 'error';
type ExportFormat = 'markdown' | 'json';
type NotesTab = 'draft' | 'final';

interface Capabilities {
  microphone_available: boolean;
  system_audio_available: boolean;
}

interface HealthState {
  connection: 'demo' | 'connected' | 'reconnecting' | 'error';
  message: string;
  latencyMs?: number;
}

interface FinalizationProgress {
  stage: string;
  percent: number;
}

const languageLabels: Record<Language, string> = {
  english: 'English',
  filipino: 'Filipino',
  taglish: 'Taglish',
};

const DEMO_SEGMENTS: TranscriptSegment[] = [
  {
    id: 'demo-0',
    sequence: 0,
    startSeconds: 0,
    endSeconds: 7,
    text: 'Welcome back. Let’s align on the beta launch and customer update.',
    provisional: false,
    speaker: 'Mia',
  },
  {
    id: 'demo-1',
    sequence: 1,
    startSeconds: 7,
    endSeconds: 14,
    text: 'We agreed to ship the beta on Friday, with a smaller onboarding cohort.',
    provisional: true,
    speaker: 'Jon',
  },
  {
    id: 'demo-2',
    sequence: 2,
    startSeconds: 14,
    endSeconds: 21,
    text: 'I will send the customer update and schedule a follow-up next week.',
    provisional: true,
    speaker: 'Mia',
  },
];

const DEMO_CAPABILITIES: Capabilities = {
  microphone_available: true,
  system_audio_available: true,
};

const initialHealth = (): HealthState => ({
  connection: 'demo',
  message: 'Demo mode · deterministic local events',
});

function errorMessage(reason: unknown): string {
  if (reason instanceof Error) return reason.message;
  if (typeof reason === 'string') return reason;
  if (reason && typeof reason === 'object' && 'message' in reason) {
    return String((reason as { message: unknown }).message);
  }
  return 'The desktop bridge could not complete that action.';
}

function formatBytes(value: number): string {
  if (value < 1024 * 1024) return `${Math.round(value / 1024)} KB`;
  if (value < 1024 * 1024 * 1024) return `${(value / (1024 * 1024)).toFixed(1)} MB`;
  return `${(value / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

function asCapabilities(value: unknown): Capabilities | null {
  if (!value || typeof value !== 'object') return null;
  const candidate = value as Partial<Capabilities>;
  if (typeof candidate.microphone_available !== 'boolean' || typeof candidate.system_audio_available !== 'boolean') return null;
  return {
    microphone_available: candidate.microphone_available,
    system_audio_available: candidate.system_audio_available,
  };
}

function asNotes(value: unknown): MeetingNotes | null {
  if (!value || typeof value !== 'object') return null;
  const candidate = value as Partial<MeetingNotes>;
  if (candidate.status !== 'draft' && candidate.status !== 'final') return null;
  if (typeof candidate.summary !== 'string' || !Array.isArray(candidate.decisions) || !Array.isArray(candidate.actionItems) || !Array.isArray(candidate.openQuestions)) return null;
  return candidate as MeetingNotes;
}

function asProgress(value: unknown): FinalizationProgress | null {
  if (!value || typeof value !== 'object') return null;
  const candidate = value as Partial<FinalizationProgress>;
  if (typeof candidate.stage !== 'string' || typeof candidate.percent !== 'number') return null;
  return { stage: candidate.stage, percent: Math.max(0, Math.min(100, candidate.percent)) };
}

function useDemoTranscript(
  state: SessionState,
  setTranscript: React.Dispatch<React.SetStateAction<TranscriptSegment[]>>,
  enabled: boolean,
) {
  useEffect(() => {
    if (!enabled || state !== 'recording') return undefined;
    let next = 0;
    const timer = window.setInterval(() => {
      const segment = DEMO_SEGMENTS[next];
      if (!segment) {
        window.clearInterval(timer);
        return;
      }
      setTranscript((current) => mergeTranscriptSegments(current, [segment]));
      next += 1;
    }, 1_100);
    return () => window.clearInterval(timer);
  }, [setTranscript, state]);
}

function NoteList({
  items,
  emptyText,
  editable = false,
  onItemChange,
}: {
  items: readonly NoteItem[];
  emptyText: string;
  editable?: boolean;
  onItemChange?: (index: number, text: string) => void;
}) {
  if (items.length === 0) return <p className="empty-note">{emptyText}</p>;
  return (
    <ul className="note-list">
      {items.map((item, index) => (
        <li key={`${item.text}-${index}`}>
          {editable ? <textarea value={item.text} onChange={(event) => onItemChange?.(index, event.target.value)} rows={2} aria-label={`Draft note ${index + 1}`} /> : <span>{item.text}</span>}
          {item.citation && <small className="citation">{formatCitation(item.citation)}</small>}
        </li>
      ))}
    </ul>
  );
}

function NotesPanel({
  draft,
  final,
  activeTab,
  onTabChange,
  onDraftChange,
  onDraftItemChange,
  finalization,
}: {
  draft: MeetingNotes;
  final: MeetingNotes | null;
  activeTab: NotesTab;
  onTabChange: (tab: NotesTab) => void;
  onDraftChange: (summary: string) => void;
  onDraftItemChange: (section: keyof Pick<MeetingNotes, 'decisions' | 'actionItems' | 'openQuestions'>, index: number, text: string) => void;
  finalization: FinalizationProgress | null;
}) {
  const visibleNotes = activeTab === 'draft' ? draft : final;
  const isDraft = activeTab === 'draft';

  return (
    <section className="notes-panel" aria-label="Meeting notes">
      <div className="panel-heading">
        <div>
          <div className="section-kicker">NOTES REVIEW</div>
          <h2>Turn talk into action</h2>
        </div>
        <div className="notes-tabs" role="tablist" aria-label="Notes version">
          <button className={activeTab === 'draft' ? 'tab-button active-tab' : 'tab-button'} type="button" role="tab" aria-selected={activeTab === 'draft'} onClick={() => onTabChange('draft')}>Draft</button>
          <button className={activeTab === 'final' ? 'tab-button active-tab' : 'tab-button'} type="button" role="tab" aria-selected={activeTab === 'final'} onClick={() => onTabChange('final')} disabled={!final}>Final</button>
        </div>
      </div>

      {isDraft && <div className="draft-callout"><span className="draft-icon">✦</span><span><strong>Draft notes</strong> update from the live transcript. They are not final until review is ready.</span></div>}
      {activeTab === 'final' && !final && <p className="empty-state">Final notes will appear after the recording is stopped and reviewed.</p>}
      {finalization && <div className="progress-wrap" aria-live="polite"><div className="progress-label"><span>{finalization.stage}</span><span>{finalization.percent}%</span></div><div className="progress-track"><span style={{ width: `${finalization.percent}%` }} /></div></div>}

      {visibleNotes && (
        <div className="notes-content">
          <label className="notes-summary"><span>Summary <small>{isDraft ? 'editable draft' : 'reviewable final'}</small></span><textarea value={visibleNotes.summary} onChange={(event) => isDraft && onDraftChange(event.target.value)} readOnly={!isDraft} rows={3} aria-label={`${isDraft ? 'Draft' : 'Final'} summary`} /></label>
          <div className="note-columns">
            <div className="note-section"><h3>Decisions</h3><NoteList items={visibleNotes.decisions} emptyText="No decisions cited yet." editable={isDraft} onItemChange={(index, text) => onDraftItemChange('decisions', index, text)} /></div>
            <div className="note-section"><h3>Action items</h3><NoteList items={visibleNotes.actionItems} emptyText="No action items cited yet." editable={isDraft} onItemChange={(index, text) => onDraftItemChange('actionItems', index, text)} /></div>
            <div className="note-section"><h3>Open questions</h3><NoteList items={visibleNotes.openQuestions} emptyText="No open questions cited yet." editable={isDraft} onItemChange={(index, text) => onDraftItemChange('openQuestions', index, text)} /></div>
          </div>
        </div>
      )}
    </section>
  );
}

export default function App() {
  const [title, setTitle] = useState('Weekly sync');
  const [language, setLanguage] = useState<Language>('english');
  const [consent, setConsent] = useState(false);
  const [state, setState] = useState<SessionState>('setup');
  const [elapsed, setElapsed] = useState(0);
  const [transcript, setTranscript] = useState<TranscriptSegment[]>([]);
  const [capabilities, setCapabilities] = useState<Capabilities | null>(null);
  const [status, setStatus] = useState('Checking capabilities');
  const [error, setError] = useState<string | null>(null);
  const [health, setHealth] = useState<HealthState>(initialHealth());
  const [draftNotes, setDraftNotes] = useState<MeetingNotes>(buildDraftNotes([]));
  const [draftEdited, setDraftEdited] = useState(false);
  const [finalNotes, setFinalNotes] = useState<MeetingNotes | null>(null);
  const [notesTab, setNotesTab] = useState<NotesTab>('draft');
  const [finalization, setFinalization] = useState<FinalizationProgress | null>(null);
  const [exporting, setExporting] = useState<ExportFormat | null>(null);
  const [deleting, setDeleting] = useState(false);
  const [deleted, setDeleted] = useState(false);
  const [modelStatus, setModelStatus] = useState<ModelStatus | null>(null);
  const [modelDownloading, setModelDownloading] = useState(false);
  const [modelManifest, setModelManifest] = useState<{ total_bytes: number } | null>(null);

  const hasNativeBridge = typeof window !== 'undefined' && Boolean(window.__TAURI__?.core?.invoke);

  useEffect(() => () => {
    if (hasNativeBridge) void invokeOrDemo('shutdown');
  }, [hasNativeBridge]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    void listenToEvents([UI_EVENTS.closeRequested], () => {
      void invokeOrDemo('shutdown').finally(() => window.close());
    }).then((cleanup) => {
      unlisten = cleanup;
    });
    return () => unlisten?.();
  }, []);

  const displayTitle = title.trim() || 'Untitled meeting';
  const setupLocked = state !== 'setup';
  const canRecord = consent && state === 'setup' && !deleted;
  const modelReady = !hasNativeBridge || modelStatus?.state === 'ready';
  const modelBusy = modelDownloading || modelStatus?.state === 'downloading';
  const canExport = Boolean(finalNotes) && !exporting && !deleting && !deleted;
  const canDelete = state === 'stopped' || state === 'ready' || state === 'error';
  const microphoneStatus = capabilities ? (capabilities.microphone_available ? 'Available · opens on Record' : 'Unavailable · check permissions') : status;
  const systemStatus = capabilities ? (capabilities.system_audio_available ? 'Available · opens on Record' : 'Unavailable · Screen Recording permission') : status;

  useEffect(() => {
    let active = true;
    const invoke = window.__TAURI__?.core?.invoke;
    if (!invoke) {
      setCapabilities(DEMO_CAPABILITIES);
      setStatus('Demo mode · devices stay idle');
      return () => { active = false; };
    }
    invoke('capabilities')
      .then((value) => {
        if (!active) return;
        const next = asCapabilities(value);
        if (next) {
          setCapabilities(next);
          setStatus('Ready to configure · devices stay idle');
          setHealth({ connection: 'connected', message: 'Desktop bridge connected · idle' });
        } else {
          setStatus('Unavailable · invalid capability response');
          setHealth({ connection: 'error', message: 'Desktop bridge returned an invalid capability response.' });
        }
      })
      .catch((reason) => {
        if (!active) return;
        setStatus('Unavailable · check macOS permissions');
        setHealth({ connection: 'error', message: errorMessage(reason) });
      });
    return () => { active = false; };
  }, []);

  useEffect(() => {
    if (!hasNativeBridge) return undefined;
    let active = true;
    invokeOrDemo('model_status')
      .then((value) => {
        if (active && value && typeof value === 'object') setModelStatus(value as ModelStatus);
      })
      .catch((reason) => {
        if (active) setError(`Model status unavailable: ${errorMessage(reason)}`);
      });
    invokeOrDemo('model_manifest')
      .then((value) => {
        if (active && value && typeof value === 'object' && 'total_bytes' in value) {
          setModelManifest(value as { total_bytes: number });
        }
      })
      .catch(() => undefined);
    let cleanup: (() => void) | undefined;
    let errorCleanup: (() => void) | undefined;
    listenToEvent(UI_EVENTS.modelProgress, (payload) => {
      if (active && payload && typeof payload === 'object') {
        const next = payload as ModelStatus;
        setModelStatus(next);
        setModelDownloading(next.state === 'downloading');
      }
    }).then((unlisten) => {
      cleanup = unlisten;
    });
    listenToEvent(UI_EVENTS.modelError, (payload) => {
      if (!active) return;
      const message = errorMessage(payload);
      setModelDownloading(false);
      setModelStatus((current) => current ? { ...current, state: 'error', error: message } : current);
      setError(`Model download failed: ${message}`);
    }).then((unlisten) => {
      errorCleanup = unlisten;
    });
    return () => {
      active = false;
      cleanup?.();
      errorCleanup?.();
    };
  }, [hasNativeBridge]);

  useEffect(() => {
    let active = true;
    let cleanup: (() => void) | undefined;
    const eventNames = Object.values(UI_EVENTS) as UiEventName[];
    listenToEvents(eventNames, (name, payload) => {
      if (!active) return;
      if (name === UI_EVENTS.transcript || name === UI_EVENTS.transcriptUpdate) {
        setTranscript((current) => {
          const segment = normalizeTranscriptPayload(payload, current.length);
          return mergeTranscriptSegments(current, [segment]);
        });
      } else if (name === UI_EVENTS.liveNotes) {
        const notes = asNotes(payload);
        if (notes) {
          setDraftNotes(notes.status === 'draft' ? notes : { ...notes, status: 'draft' });
          setDraftEdited(false);
        }
      } else if (name === UI_EVENTS.finalNotes) {
        const notes = asNotes(payload);
        if (notes) {
          setFinalNotes(notes.status === 'final' ? notes : { ...notes, status: 'final' });
          setFinalization(null);
          setNotesTab('final');
          setState('ready');
        }
      } else if (name === UI_EVENTS.session) {
        const session = typeof payload === 'string' ? payload : payload && typeof payload === 'object' && 'state' in payload ? String((payload as { state: unknown }).state) : '';
        if (session === 'listening' || session === 'recording') setState('recording');
        if (session === 'paused') setState('paused');
        if (session === 'stopped') setState('stopped');
        if (session === 'processing' || session === 'finalizing') setState('processing');
        if (session === 'ready') setState('ready');
      } else if (name === UI_EVENTS.health) {
        if (payload && typeof payload === 'object') {
          const value = payload as { message?: unknown; latency_ms?: unknown; latencyMs?: unknown; status?: unknown };
          setHealth({
            connection: value.status === 'reconnecting' ? 'reconnecting' : 'connected',
            message: typeof value.message === 'string' ? value.message : 'Desktop event stream connected',
            latencyMs: typeof value.latency_ms === 'number' ? value.latency_ms : typeof value.latencyMs === 'number' ? value.latencyMs : undefined,
          });
        }
      } else if (name === UI_EVENTS.finalization) {
        const progress = asProgress(payload);
        if (progress) setFinalization(progress);
      } else if (name === UI_EVENTS.error) {
        setError(errorMessage(payload));
        setState('error');
      }
    }).then((unlisten) => {
      cleanup = unlisten;
    }).catch((reason) => {
      if (active && hasNativeBridge) setHealth({ connection: 'error', message: errorMessage(reason) });
    });
    return () => {
      active = false;
      cleanup?.();
    };
  }, [hasNativeBridge]);

  useEffect(() => {
    if (state !== 'recording') return undefined;
    const interval = window.setInterval(() => setElapsed((value) => value + 1), 1000);
    return () => window.clearInterval(interval);
  }, [state]);

  useEffect(() => {
    if (!draftEdited) setDraftNotes(buildDraftNotes(transcript));
  }, [draftEdited, transcript]);

  useEffect(() => {
    if (state !== 'processing' || !hasNativeBridge) return undefined;
    const timer = window.setInterval(() => {
      invokeOrDemo('session_state')
        .then((value) => {
          if (!value || typeof value !== 'object') return;
          const next = value as { state?: unknown; final_transcript?: unknown; final_notes?: unknown };
          if (next.final_transcript && typeof next.final_transcript === 'object' && Array.isArray((next.final_transcript as { segments?: unknown }).segments)) {
            setTranscript((next.final_transcript as { segments: unknown[] }).segments.map((segment, index) => normalizeTranscriptPayload(segment, index)));
          }
          const notes = asNotes(next.final_notes);
          if (notes) {
            setFinalNotes(notes.status === 'final' ? notes : { ...notes, status: 'final' });
            setFinalization(null);
            setNotesTab('final');
            setState('ready');
          } else if (next.state === 'stopped') {
            setFinalization({ stage: 'Finalizing transcript and notes…', percent: 50 });
          }
        })
        .catch(() => undefined);
    }, 1_000);
    return () => window.clearInterval(timer);
  }, [hasNativeBridge, state]);

  useDemoTranscript(state, setTranscript, !hasNativeBridge);

  const timer = useMemo(() => {
    const minutes = Math.floor(elapsed / 60);
    const seconds = elapsed % 60;
    return `${String(minutes).padStart(2, '0')}:${String(seconds).padStart(2, '0')}`;
  }, [elapsed]);

  const invokeAction = async (command: string, args?: Record<string, unknown>) => {
    setError(null);
    return invokeOrDemo(command, args);
  };

  const startRecording = async () => {
    if (!canRecord || !modelReady) {
      if (!modelReady) setError('Download the WhisperX model before recording.');
      return;
    }
    setError(null);
    try {
      await invokeAction('create_session', { language, title: displayTitle });
      await invokeAction('accept_consent');
      await invokeAction('record');
      setTranscript([]);
      setFinalNotes(null);
      setDraftEdited(false);
      setDraftNotes(buildDraftNotes([]));
      setFinalization(null);
      setElapsed(0);
      setNotesTab('draft');
      setHealth(hasNativeBridge ? { connection: 'connected', message: 'Listening · live events connected' } : initialHealth());
      setState('recording');
    } catch (reason) {
      setError(errorMessage(reason));
      setHealth({ connection: 'error', message: 'Recording did not start.' });
      setState('setup');
    }
  };

  const downloadModel = async () => {
    if (modelBusy || !hasNativeBridge) return;
    setModelDownloading(true);
    setModelStatus((current) => current ? { ...current, state: 'downloading', error: null } : current);
    setError(null);
    try {
      await invokeAction('download_model');
    } catch (reason) {
      setError(`Model download failed: ${errorMessage(reason)}`);
      setModelStatus((current) => current ? { ...current, state: 'error', error: errorMessage(reason) } : current);
    } finally {
      const status = await invokeOrDemo('model_status').catch(() => undefined);
      if (status && typeof status === 'object') {
        const next = status as ModelStatus;
        setModelStatus(next);
        setModelDownloading(next.state === 'downloading');
      }
    }
  };

  const recoverModel = async () => {
    if (!hasNativeBridge) return;
    try {
      const value = await invokeAction('model_recover');
      if (value && typeof value === 'object') setModelStatus(value as ModelStatus);
    } catch (reason) {
      setError(`Could not recover model setup: ${errorMessage(reason)}`);
    }
  };

  const cancelModelDownload = async () => {
    try {
      await invokeAction('cancel_model_download');
    } catch (reason) {
      setError(`Could not cancel model download: ${errorMessage(reason)}`);
    }
  };

  const pauseOrResume = async () => {
    if (state !== 'paused' && state !== 'recording') return;
    const command = state === 'paused' ? 'resume' : 'pause';
    try {
      await invokeAction(command);
      setState(state === 'paused' ? 'recording' : 'paused');
      setHealth(hasNativeBridge ? { connection: 'connected', message: state === 'paused' ? 'Listening resumed' : 'Capture paused' } : initialHealth());
    } catch (reason) {
      setError(errorMessage(reason));
      setHealth({ connection: 'error', message: 'Capture control failed.' });
    }
  };

  const stopRecording = async () => {
    if (state !== 'paused' && state !== 'recording') return;
    try {
      await invokeAction('stop');
      setState(hasNativeBridge ? 'processing' : 'stopped');
      setFinalization(hasNativeBridge ? { stage: 'Finalizing transcript and notes…', percent: 25 } : null);
      setHealth(hasNativeBridge ? { connection: 'connected', message: 'Recording sealed · final review in progress' } : { connection: 'demo', message: 'Demo recording sealed locally' });
      if (!hasNativeBridge) {
        const authoritative = transcript.map((segment) => ({ ...segment, provisional: false }));
        setTranscript(authoritative);
        setFinalNotes(buildFinalNotes(authoritative));
        setNotesTab('final');
        setState('ready');
      }
    } catch (reason) {
      setError(errorMessage(reason));
      setState('error');
      setHealth({ connection: 'error', message: 'Capture stopped with an error.' });
    }
  };

  const exportMeeting = async (format: ExportFormat) => {
    if (!canExport) return;
    setExporting(format);
    setError(null);
    try {
      const payload = serializeMeetingExport({ title: displayTitle, transcript, notes: finalNotes ?? buildFinalNotes(transcript) }, format);
      if (hasNativeBridge) {
        await invokeAction('export_meeting', {
          destination: `meeting-${Date.now()}`,
          format,
        });
      } else {
        const blob = new Blob([payload], { type: format === 'json' ? 'application/json' : 'text/markdown' });
        const url = URL.createObjectURL(blob);
        const anchor = document.createElement('a');
        anchor.href = url;
        anchor.download = `${displayTitle.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '') || 'meeting-notes'}.${format === 'json' ? 'json' : 'md'}`;
        anchor.click();
        URL.revokeObjectURL(url);
      }
    } catch (reason) {
      setError(`Export failed: ${errorMessage(reason)}`);
    } finally {
      setExporting(null);
    }
  };

  const deleteMeeting = async () => {
    if (!canDelete || deleting) return;
    if (!window.confirm('Delete this meeting, its notes, transcript, and retained recording? This cannot be undone.')) return;
    setDeleting(true);
    setError(null);
    try {
      if (hasNativeBridge) await invokeAction('delete_meeting');
      setDeleted(true);
      setState('setup');
      setTranscript([]);
      setFinalNotes(null);
      setDraftEdited(false);
      setDraftNotes(buildDraftNotes([]));
      setConsent(false);
      setElapsed(0);
      setFinalization(null);
      setHealth({ connection: hasNativeBridge ? 'connected' : 'demo', message: 'Meeting deleted. Nothing is listening.' });
    } catch (reason) {
      setError(`Delete failed: ${errorMessage(reason)}`);
    } finally {
      setDeleting(false);
    }
  };

  const updateDraftSummary = (summary: string) => {
    setDraftEdited(true);
    setDraftNotes((current) => ({ ...current, summary }));
  };
  const updateDraftItem = (section: keyof Pick<MeetingNotes, 'decisions' | 'actionItems' | 'openQuestions'>, index: number, text: string) => {
    setDraftEdited(true);
    setDraftNotes((current) => ({
      ...current,
      [section]: current[section].map((item, itemIndex) => itemIndex === index ? { ...item, text } : item),
    }));
  };
  const statusLabel = state === 'setup' ? 'Not recording' : state === 'paused' ? 'Paused' : state === 'stopped' ? 'Stopped' : state === 'processing' ? 'Finalizing' : state === 'ready' ? 'Ready for review' : state === 'error' ? 'Needs attention' : 'Recording';

  return (
    <main className="shell">
      <header className="topbar">
        <div className="brand"><span className="brand-mark" aria-hidden="true">✦</span><span>Meeting Notes</span></div>
        <div className="privacy-pill"><span className="dot" /> Local-first</div>
      </header>

      <section className="hero">
        <div className="eyebrow">LIVE MEETING WORKSPACE</div>
        <h1>Stay present.<br /><em>Remember everything.</em></h1>
        <p className="hero-copy">Capture your microphone and system audio locally, then turn the conversation into clear notes with WhisperX.</p>
      </section>

      <section className="workspace-card" aria-label="New meeting">
        <div className="card-header">
          <div><div className="section-kicker">NEW MEETING</div><h2>Set up your session</h2></div>
          <div className={`idle-badge ${state !== 'setup' ? 'active-badge' : ''}`}><span className="idle-dot" /> {statusLabel}</div>
        </div>

        <div className="form-grid">
          <label className="field field-wide"><span>Meeting title</span><input type="text" value={title} onChange={(event) => setTitle(event.target.value)} disabled={setupLocked || deleted} /></label>
          <label className="field"><span>Transcript language</span><select value={language} onChange={(event) => setLanguage(event.target.value as Language)} disabled={setupLocked || deleted}>{Object.entries(languageLabels).map(([value, label]) => <option key={value} value={value}>{label}</option>)}</select></label>
        </div>

        <div className="sources">
          <div className="source-heading"><div><div className="section-kicker">CAPTURE SOURCES</div><p>Audio devices stay idle until you press Record.</p></div><span className={`ready-label ${health.connection === 'error' ? 'health-error' : ''}`}><span className="ready-dot" /> {status}</span></div>
          <div className="source-grid">
            <div className="source-row"><div className="source-icon mic-icon">♩</div><div className="source-info"><strong>Microphone</strong><span>{microphoneStatus}</span></div><button className="source-action" type="button" disabled={setupLocked || deleted}>Configure <span>›</span></button></div>
            <div className="source-row"><div className="source-icon system-icon">◉</div><div className="source-info"><strong>System audio</strong><span>{systemStatus}</span></div><button className="source-action" type="button" disabled={setupLocked || deleted}>Configure <span>›</span></button></div>
          </div>
        </div>

        <label className="consent-row"><input type="checkbox" checked={consent} onChange={(event) => setConsent(event.target.checked)} disabled={setupLocked || deleted} /><span className="checkmark">✓</span><span>I have consent to record this meeting and understand audio is stored locally.</span></label>
        <div className="model-setup" aria-live="polite"><div><div className="section-kicker">WHISPERX MODEL</div><strong>{modelStatus?.state === 'ready' ? 'Ready for offline transcription' : modelStatus?.state === 'downloading' ? `Downloading ${modelStatus.current_asset ?? 'model files'}…` : 'Download the local transcription model'}</strong><span>{modelStatus?.state === 'ready' ? 'The model is installed locally. Recording will not download anything.' : `Required once before your first recording${modelManifest ? ` · ${formatBytes(modelManifest.total_bytes)}` : ''}.`}</span></div><div className="model-actions">{!hasNativeBridge ? <span className="model-percent">Desktop app required</span> : modelStatus?.state === 'ready' ? <span className="ready-label"><span className="ready-dot" /> Ready</span> : modelBusy ? <><span className="model-percent">{modelStatus?.percent ?? 0}%</span><button className="source-action" type="button" onClick={cancelModelDownload}>Cancel</button><button className="source-action" type="button" onClick={recoverModel}>Recover</button></> : <button className="secondary-button" type="button" onClick={downloadModel}>Download model</button>}</div>{modelBusy && <div className="progress-track model-progress"><span style={{ width: `${modelStatus?.percent ?? 0}%` }} /></div>}{modelStatus?.state === 'error' && <p className="error-note">{modelStatus.error ?? 'The model could not be installed. Retry the download.'}</p>}</div>

        <button className="record-button" type="button" disabled={!canRecord || !modelReady} onClick={startRecording}><span className="record-icon" /> {modelReady ? 'Record meeting' : 'Download model to record'}</button>
        <p className="privacy-note"><span>⌁</span> Nothing is captured, processed, or saved before you press Record.</p>
        {error && <p className="error-note" role="alert">{error}</p>}
        {deleted && <p className="success-note" role="status">Meeting deleted. Audio and notes were removed from this workspace.</p>}
      </section>

      {state !== 'setup' && (
        <section className="live-panel" aria-live="polite">
          <div className="live-header"><div><div className="section-kicker">{state === 'processing' || state === 'ready' ? 'FINAL REVIEW' : 'LIVE SESSION'}</div><h2>{displayTitle}</h2></div><div className="live-time"><span className={`recording-dot ${state === 'recording' ? '' : 'paused-dot'}`} />{timer}</div></div>
          <div className="live-body">
            <div className="transcript-heading"><div><h3>Transcript</h3><span>{state === 'recording' || state === 'paused' ? 'Live · provisional updates may change' : state === 'processing' ? 'Final transcript in progress' : 'Final transcript · editable review view'}</span></div><div className="connection-state"><span className={`connection-dot ${health.connection}`} />{health.message}{health.latencyMs !== undefined && ` · ${health.latencyMs} ms`}</div></div>
            <div className="transcript-list">
              {transcript.length === 0 && <div className="transcript-empty"><span className="pulse-line" />{state === 'processing' ? 'Preparing the final transcript…' : state === 'ready' ? 'No transcript was captured.' : state === 'paused' ? 'Capture paused. Resume when ready.' : 'Live transcript will appear here.'}</div>}
              {transcript.map((segment) => <article className={`transcript-segment ${segment.provisional ? 'provisional' : ''}`} key={segment.id}><div className="segment-meta"><span>{formatCitation({ startSeconds: segment.startSeconds, endSeconds: segment.endSeconds })}</span>{segment.speaker && <span>{segment.speaker}</span>}{segment.provisional && <span className="provisional-label">Draft</span>}</div><p>{segment.text}</p></article>)}
            </div>
            <div className="live-actions"><button className="secondary-button" type="button" disabled={state !== 'recording' && state !== 'paused'} onClick={pauseOrResume}>{state === 'paused' ? 'Resume' : 'Pause'}</button><button className="stop-button" type="button" disabled={state !== 'recording' && state !== 'paused'} onClick={stopRecording}>Stop recording</button></div>
          </div>
        </section>
      )}

      {(state === 'recording' || state === 'paused' || state === 'stopped' || state === 'processing' || state === 'ready' || state === 'error') && (
        <NotesPanel draft={draftNotes} final={finalNotes} activeTab={notesTab} onTabChange={setNotesTab} onDraftChange={updateDraftSummary} onDraftItemChange={updateDraftItem} finalization={finalization} />
      )}

      {(state === 'stopped' || state === 'processing' || state === 'ready' || state === 'error') && (
        <section className="review-actions" aria-label="Meeting actions">
          <div><div className="section-kicker">MEETING CONTROLS</div><h2>{state === 'processing' ? 'Final review is processing' : state === 'error' ? 'Review needs attention' : 'Keep, share, or remove this review'}</h2><p>{state === 'processing' ? 'Exports and deletion become available when finalization finishes.' : 'Exports use the reviewed transcript and final notes. Deleting removes the retained local recording too.'}</p></div>
          <div className="action-group"><button className="secondary-button" type="button" disabled={!canExport} onClick={() => exportMeeting('markdown')}>{exporting === 'markdown' ? 'Exporting…' : 'Export Markdown'}</button><button className="secondary-button" type="button" disabled={!canExport} onClick={() => exportMeeting('json')}>{exporting === 'json' ? 'Exporting…' : 'Export JSON'}</button><button className="delete-button" type="button" disabled={!canDelete || deleting} onClick={deleteMeeting}>{deleting ? 'Deleting…' : 'Delete meeting'}</button></div>
        </section>
      )}

      <footer><span>WhisperX-ready</span><span>•</span><span>Audio stays on this Mac</span><span>•</span><span>{hasNativeBridge ? 'Desktop bridge' : 'Demo fallback'}</span></footer>
    </main>
  );
}
