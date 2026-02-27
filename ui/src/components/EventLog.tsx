/** Collapsible system-wide activity feed displaying timestamped events with color-coded source tags. */
import { useState, useRef, useEffect } from 'react';
import type { EventLogEntry } from '../types';

/** Color map for event source tags, using CSS custom properties. */
const SOURCE_COLORS: Record<EventLogEntry['source'], string> = {
  agent: 'var(--color-accent)',
  phase: 'var(--color-info)',
  review: '#a371f7',
  system: 'var(--color-text-secondary)',
  error: 'var(--color-error)',
  git: 'var(--color-warning)',
};

/** Props for the EventLog component. */
export interface EventLogProps {
  /** Array of event log entries to display. */
  entries: EventLogEntry[];
}

/**
 * Renders a collapsible event log panel with timestamped, color-coded entries.
 * Auto-scrolls to the latest entry when new events arrive.
 */
export default function EventLog({ entries }: EventLogProps): React.JSX.Element {
  const [collapsed, setCollapsed] = useState(false);
  const scrollRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!collapsed && scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [entries, collapsed]);

  const formatTime = (iso: string): string => {
    const d = new Date(iso);
    return d.toLocaleTimeString('en-US', { hour12: false });
  };

  return (
    <div style={{
      borderTop: '1px solid var(--color-border)',
      backgroundColor: 'var(--color-bg-card)',
      flexShrink: 0,
    }}>
      {/* Header */}
      <button
        onClick={() => setCollapsed(!collapsed)}
        style={{
          width: '100%',
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'space-between',
          padding: '6px 12px',
          background: 'transparent',
          border: 'none',
          borderBottom: collapsed ? 'none' : '1px solid var(--color-border)',
          color: 'var(--color-text-secondary)',
          cursor: 'pointer',
          fontSize: '11px',
          fontFamily: 'inherit',
          textTransform: 'uppercase',
          letterSpacing: '1px',
        }}
      >
        <span>Event Log ({entries.length})</span>
        <span style={{
          transform: collapsed ? 'rotate(180deg)' : 'rotate(0deg)',
          transition: 'transform 0.2s',
        }}>
          ▼
        </span>
      </button>

      {/* Log content */}
      {!collapsed && (
        <div
          ref={scrollRef}
          style={{
            height: '150px',
            overflowY: 'auto',
            padding: '4px 12px',
            fontSize: '12px',
            lineHeight: '1.8',
          }}
        >
          {entries.map(entry => (
            <div key={entry.id} style={{ display: 'flex', gap: '8px' }}>
              <span style={{ color: 'var(--color-text-secondary)', flexShrink: 0 }}>
                {formatTime(entry.timestamp)}
              </span>
              <span style={{
                color: SOURCE_COLORS[entry.source],
                flexShrink: 0,
                width: '60px',
              }}>
                [{entry.source}]
              </span>
              <span style={{ color: 'var(--color-text-primary)' }}>
                {entry.message}
              </span>
            </div>
          ))}
          {entries.length === 0 && (
            <span style={{ color: 'var(--color-text-secondary)' }}>
              No events yet...
            </span>
          )}
        </div>
      )}
    </div>
  );
}
