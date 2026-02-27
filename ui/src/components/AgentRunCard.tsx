/** Expandable agent run card — collapsed summary with status, expanded tabbed detail view. */
import { useState, useEffect, useRef } from 'react';
import type { AgentRunCard as AgentRunCardType, PipelinePhase, AgentTeamDetail, AgentEvent, PipelineStatus, AgentEventType } from '../types';

/** Tab options for the expanded detail view. */
export type DetailTab = 'output' | 'phases' | 'files';

/** Props for the AgentRunCard component. */
export interface AgentRunCardProps {
  /** Combined issue + run + project data for the card. */
  card: AgentRunCardType;
  /** Pipeline phases for the phases tab. */
  phases?: PipelinePhase[];
  /** Agent team detail with tasks. */
  agentTeam?: AgentTeamDetail;
  /** Agent events keyed by task ID. */
  agentEvents?: Map<number, AgentEvent[]>;
  /** Callback when the cancel button is clicked. */
  onCancel?: (runId: number) => void;
  /** Grid or list layout mode. */
  viewMode: 'grid' | 'list';
}

/** Status dot color mapped to CSS custom properties. */
export const STATUS_DOT_COLORS: Record<PipelineStatus, string> = {
  running: 'var(--color-success)',
  queued: 'var(--color-warning)',
  completed: 'var(--color-success)',
  failed: 'var(--color-error)',
  cancelled: 'var(--color-text-secondary)',
};

/** Human-readable status labels. */
export const STATUS_LABELS: Record<PipelineStatus, string> = {
  running: 'RUNNING',
  queued: 'QUEUED',
  completed: 'DONE',
  failed: 'FAILED',
  cancelled: 'CANCELLED',
};

/** Color mapping for agent event types in the output tab. */
const EVENT_TYPE_COLORS: Record<AgentEventType, string> = {
  thinking: 'var(--color-text-secondary)',
  action: '#39d353',
  output: 'var(--color-success)',
  signal: 'var(--color-warning)',
  error: 'var(--color-error)',
};

/** AgentRunCard displays a pipeline run as a collapsible card with output, phases, and files tabs. */
export default function AgentRunCard({
  card,
  phases,
  agentTeam,
  agentEvents,
  onCancel,
  viewMode,
}: AgentRunCardProps): React.JSX.Element {
  const { run, issue, project } = card;
  const [expanded, setExpanded] = useState(false);
  const [activeTab, setActiveTab] = useState<DetailTab>('output');
  const [elapsed, setElapsed] = useState('');
  const outputRef = useRef<HTMLDivElement>(null);

  // Live elapsed timer
  useEffect(() => {
    if (run.status !== 'running' && run.status !== 'queued') {
      if (run.started_at && run.completed_at) {
        const diff = new Date(run.completed_at).getTime() - new Date(run.started_at).getTime();
        const m = Math.floor(diff / 60000);
        const s = Math.floor((diff % 60000) / 1000);
        setElapsed(`${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}`);
      }
      return;
    }

    const start = new Date(run.started_at).getTime();
    const tick = () => {
      const diff = Date.now() - start;
      const m = Math.floor(diff / 60000);
      const s = Math.floor((diff % 60000) / 1000);
      setElapsed(`${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}`);
    };
    tick();
    const interval = setInterval(tick, 1000);
    return () => clearInterval(interval);
  }, [run.status, run.started_at, run.completed_at]);

  // Auto-scroll output
  useEffect(() => {
    if (expanded && activeTab === 'output' && outputRef.current) {
      outputRef.current.scrollTop = outputRef.current.scrollHeight;
    }
  });

  // Collect all events for this run's agent team
  const allEvents: AgentEvent[] = [];
  if (agentTeam && agentEvents) {
    for (const task of agentTeam.tasks) {
      const events = agentEvents.get(task.id) ?? [];
      allEvents.push(...events);
    }
    allEvents.sort((a, b) => new Date(a.created_at).getTime() - new Date(b.created_at).getTime());
  }

  const progress = run.phase_count
    ? Math.round(((run.current_phase ?? 0) / run.phase_count) * 100)
    : 0;

  return (
    <div
      data-testid="agent-run-card"
      onClick={() => setExpanded(!expanded)}
      style={{
        backgroundColor: 'var(--color-bg-card)',
        border: '1px solid var(--color-border)',
        borderLeft: `3px solid ${STATUS_DOT_COLORS[run.status] ?? 'var(--color-border)'}`,
        cursor: 'pointer',
        transition: 'background-color 0.15s',
      }}
      onMouseEnter={e => (e.currentTarget.style.backgroundColor = 'var(--color-bg-card-hover)')}
      onMouseLeave={e => (e.currentTarget.style.backgroundColor = 'var(--color-bg-card)')}
    >
      {/* Collapsed view */}
      <div style={{
        display: 'flex',
        alignItems: 'center',
        padding: '12px',
        gap: '12px',
      }}>
        {/* Status dot */}
        <span
          data-testid="status-dot"
          className={run.status === 'running' ? 'pulse-dot' : undefined}
          style={{
            width: '8px',
            height: '8px',
            borderRadius: '50%',
            backgroundColor: STATUS_DOT_COLORS[run.status],
            flexShrink: 0,
          }}
        />

        {/* Project badge + title */}
        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
            <span style={{
              fontSize: '10px',
              padding: '1px 6px',
              backgroundColor: 'var(--color-border)',
              color: 'var(--color-text-secondary)',
              textTransform: 'uppercase',
              letterSpacing: '0.5px',
              flexShrink: 0,
            }}>
              {project.name}
            </span>
            <span style={{
              overflow: 'hidden',
              textOverflow: 'ellipsis',
              whiteSpace: 'nowrap',
              fontSize: '13px',
            }}>
              {issue.title}
            </span>
          </div>
        </div>

        {/* Phase dots */}
        {run.phase_count && (
          <div style={{ display: 'flex', gap: '3px', flexShrink: 0 }}>
            {Array.from({ length: run.phase_count }, (_, i) => {
              const phaseNum = i + 1;
              const isCurrent = phaseNum === (run.current_phase ?? 0);
              const isDone = phaseNum < (run.current_phase ?? 0);
              return (
                <span
                  key={i}
                  data-testid="phase-dot"
                  className={isCurrent && run.status === 'running' ? 'pulse-dot' : undefined}
                  style={{
                    width: '6px',
                    height: '6px',
                    borderRadius: '50%',
                    backgroundColor: isDone
                      ? 'var(--color-success)'
                      : isCurrent
                        ? 'var(--color-info)'
                        : 'var(--color-border)',
                  }}
                />
              );
            })}
          </div>
        )}

        {/* Status label */}
        <span style={{
          fontSize: '11px',
          color: STATUS_DOT_COLORS[run.status],
          fontWeight: 600,
          flexShrink: 0,
          width: '60px',
          textAlign: 'right',
        }}>
          {STATUS_LABELS[run.status]}
        </span>

        {/* Elapsed */}
        <span style={{
          fontSize: '12px',
          color: 'var(--color-text-secondary)',
          flexShrink: 0,
          width: '50px',
          textAlign: 'right',
          fontVariantNumeric: 'tabular-nums',
        }}>
          {elapsed}
        </span>

        {/* Expand chevron */}
        <span style={{
          color: 'var(--color-text-secondary)',
          transform: expanded ? 'rotate(180deg)' : 'rotate(0deg)',
          transition: 'transform 0.2s',
          flexShrink: 0,
        }}>
          &#x25BC;
        </span>
      </div>

      {/* Progress bar */}
      {run.status === 'running' && (
        <div style={{
          height: '2px',
          backgroundColor: 'var(--color-border)',
        }}>
          <div
            data-testid="progress-bar"
            style={{
              height: '100%',
              width: `${progress}%`,
              backgroundColor: 'var(--color-success)',
              transition: 'width 0.3s',
            }}
          />
        </div>
      )}

      {/* Expanded detail view */}
      {expanded && (
        <div
          onClick={e => e.stopPropagation()}
          style={{ borderTop: '1px solid var(--color-border)' }}
        >
          {/* Tabs */}
          <div style={{
            display: 'flex',
            borderBottom: '1px solid var(--color-border)',
          }}>
            {(['output', 'phases', 'files'] as DetailTab[]).map(tab => (
              <button
                key={tab}
                onClick={() => setActiveTab(tab)}
                style={{
                  padding: '8px 16px',
                  background: 'transparent',
                  border: 'none',
                  borderBottom: activeTab === tab ? '2px solid var(--color-success)' : '2px solid transparent',
                  color: activeTab === tab ? 'var(--color-text-primary)' : 'var(--color-text-secondary)',
                  cursor: 'pointer',
                  fontSize: '12px',
                  fontFamily: 'inherit',
                  textTransform: 'uppercase',
                  letterSpacing: '0.5px',
                }}
              >
                {tab}
              </button>
            ))}

            {/* Cancel button for running pipelines */}
            {run.status === 'running' && onCancel && (
              <button
                onClick={() => onCancel(run.id)}
                style={{
                  marginLeft: 'auto',
                  padding: '8px 16px',
                  background: 'transparent',
                  border: 'none',
                  color: 'var(--color-error)',
                  cursor: 'pointer',
                  fontSize: '12px',
                  fontFamily: 'inherit',
                }}
              >
                cancel
              </button>
            )}
          </div>

          {/* Tab content */}
          <div style={{ padding: '12px', maxHeight: '400px', overflow: 'hidden' }}>
            {activeTab === 'output' && (
              <div
                ref={outputRef}
                style={{
                  backgroundColor: '#000',
                  padding: '12px',
                  maxHeight: '376px',
                  overflowY: 'auto',
                  fontSize: '12px',
                  lineHeight: '1.6',
                }}
              >
                {allEvents.length === 0 ? (
                  <span style={{ color: 'var(--color-text-secondary)' }}>
                    {run.status === 'queued' ? 'Waiting to start...' : 'No output yet...'}
                  </span>
                ) : (
                  allEvents.map((event, i) => (
                    <div key={i} style={{ color: EVENT_TYPE_COLORS[event.event_type] ?? 'var(--color-text-primary)' }}>
                      <span style={{ color: 'var(--color-text-secondary)', marginRight: '8px' }}>
                        [{event.event_type}]
                      </span>
                      {event.content}
                    </div>
                  ))
                )}
              </div>
            )}

            {activeTab === 'phases' && (
              <div style={{ fontSize: '13px' }}>
                {(phases ?? []).length === 0 ? (
                  <span style={{ color: 'var(--color-text-secondary)' }}>No phases yet...</span>
                ) : (
                  (phases ?? []).map((phase, i) => (
                    <div key={i} style={{
                      display: 'flex',
                      alignItems: 'center',
                      gap: '12px',
                      padding: '6px 0',
                      borderBottom: '1px solid var(--color-border)',
                    }}>
                      <span style={{
                        width: '16px',
                        textAlign: 'center',
                        color: phase.status === 'completed'
                          ? 'var(--color-success)'
                          : phase.status === 'running'
                            ? 'var(--color-info)'
                            : 'var(--color-text-secondary)',
                      }}>
                        {phase.status === 'completed' ? '\u2713' : phase.status === 'running' ? '\u25B6' : '\u25CB'}
                      </span>
                      <span style={{ flex: 1 }}>{phase.phase_name}</span>
                      {phase.iteration != null && phase.budget != null && (
                        <span style={{ color: 'var(--color-text-secondary)', fontSize: '11px' }}>
                          iter {phase.iteration}/{phase.budget}
                        </span>
                      )}
                      {phase.review_status && (
                        <span style={{
                          fontSize: '10px',
                          padding: '1px 6px',
                          backgroundColor: phase.review_status === 'passed'
                            ? 'var(--color-success)'
                            : 'var(--color-warning)',
                          color: '#000',
                        }}>
                          {phase.review_status}
                        </span>
                      )}
                    </div>
                  ))
                )}
              </div>
            )}

            {activeTab === 'files' && (
              <div style={{ fontSize: '13px', color: 'var(--color-text-secondary)' }}>
                {run.branch_name ? (
                  <div style={{ marginBottom: '8px' }}>
                    <span style={{ color: 'var(--color-text-secondary)' }}>branch: </span>
                    <span style={{ color: 'var(--color-info)' }}>{run.branch_name}</span>
                  </div>
                ) : null}
                {run.pr_url ? (
                  <div>
                    <span style={{ color: 'var(--color-text-secondary)' }}>PR: </span>
                    <a
                      href={run.pr_url}
                      target="_blank"
                      rel="noopener noreferrer"
                      style={{ color: 'var(--color-info)' }}
                      onClick={e => e.stopPropagation()}
                    >
                      {run.pr_url}
                    </a>
                  </div>
                ) : null}
                {!run.branch_name && !run.pr_url && (
                  <span>No file changes yet...</span>
                )}
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
