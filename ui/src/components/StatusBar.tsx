/** Top status bar — system stats, command input, view toggle, uptime, WebSocket indicator. */
import { useState, useRef, useEffect } from 'react';
import { useWsStatus } from '../contexts/WebSocketContext';
import type { ViewMode } from '../types';

/** Props for the StatusBar component */
export interface StatusBarProps {
  /** Counts of agent runs by status */
  agentCounts: {
    running: number;
    queued: number;
    completed: number;
    failed: number;
  };
  /** Total number of projects loaded */
  projectCount: number;
  /** Callback when user submits a command via the input */
  onCommand?: (command: string) => void;
  /** Current view mode for the grid */
  viewMode: ViewMode;
  /** Callback when user toggles the view mode */
  onViewModeChange: (mode: ViewMode) => void;
}

/** Format seconds into HH:MM:SS */
function formatUptime(seconds: number): string {
  const h = Math.floor(seconds / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  const s = seconds % 60;
  return `${String(h).padStart(2, '0')}:${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}`;
}

/** Status bar displayed at the top of the Mission Control dashboard. */
export default function StatusBar({
  agentCounts,
  projectCount,
  onCommand,
  viewMode,
  onViewModeChange,
}: StatusBarProps) {
  const wsStatus = useWsStatus();
  const [commandInput, setCommandInput] = useState('');
  const [uptime, setUptime] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);

  // Uptime counter
  useEffect(() => {
    const interval = setInterval(() => setUptime(u => u + 1), 1000);
    return () => clearInterval(interval);
  }, []);

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter' && commandInput.trim()) {
      onCommand?.(commandInput.trim());
      setCommandInput('');
    }
  };

  const wsColor = wsStatus === 'connected'
    ? 'var(--color-success)'
    : wsStatus === 'connecting'
      ? 'var(--color-warning)'
      : 'var(--color-error)';

  return (
    <div style={{
      display: 'flex',
      alignItems: 'center',
      height: '40px',
      padding: '0 12px',
      backgroundColor: 'var(--color-bg-card)',
      borderBottom: '1px solid var(--color-border)',
      gap: '16px',
      fontSize: '13px',
      flexShrink: 0,
    }}>
      {/* Logo */}
      <span style={{ color: 'var(--color-success)', fontWeight: 700, letterSpacing: '2px' }}>
        FORGE
      </span>

      {/* System stats */}
      <div style={{ display: 'flex', gap: '12px', color: 'var(--color-text-secondary)' }}>
        <span>
          <span style={{ color: 'var(--color-success)' }}>{agentCounts.running}</span> running
        </span>
        <span>
          <span style={{ color: 'var(--color-warning)' }}>{agentCounts.queued}</span> queued
        </span>
        <span>
          <span style={{ color: 'var(--color-success)' }}>{agentCounts.completed}</span> done
        </span>
        <span>
          <span style={{ color: 'var(--color-error)' }}>{agentCounts.failed}</span> failed
        </span>
        <span>{projectCount} projects</span>
      </div>

      {/* Command input */}
      <div style={{
        flex: 1,
        display: 'flex',
        alignItems: 'center',
        maxWidth: '500px',
        margin: '0 auto',
      }}>
        <span style={{ color: 'var(--color-accent)', marginRight: '8px' }}>forge&gt;</span>
        <input
          ref={inputRef}
          type="text"
          value={commandInput}
          onChange={e => setCommandInput(e.target.value)}
          onKeyDown={handleKeyDown}
          placeholder="type a command..."
          style={{
            flex: 1,
            background: 'transparent',
            border: 'none',
            outline: 'none',
            color: 'var(--color-text-primary)',
            fontFamily: 'inherit',
            fontSize: 'inherit',
          }}
        />
      </div>

      {/* View toggle */}
      <div style={{ display: 'flex', gap: '4px' }}>
        <button
          onClick={() => onViewModeChange('grid')}
          style={{
            padding: '4px 8px',
            background: viewMode === 'grid' ? 'var(--color-border)' : 'transparent',
            border: '1px solid var(--color-border)',
            color: 'var(--color-text-primary)',
            cursor: 'pointer',
            fontSize: '12px',
          }}
          title="Grid view"
        >
          grid
        </button>
        <button
          onClick={() => onViewModeChange('list')}
          style={{
            padding: '4px 8px',
            background: viewMode === 'list' ? 'var(--color-border)' : 'transparent',
            border: '1px solid var(--color-border)',
            color: 'var(--color-text-primary)',
            cursor: 'pointer',
            fontSize: '12px',
          }}
          title="List view"
        >
          list
        </button>
      </div>

      {/* Uptime + WS status */}
      <span style={{ color: 'var(--color-text-secondary)' }}>
        {formatUptime(uptime)}
      </span>
      <span
        style={{
          width: '8px',
          height: '8px',
          borderRadius: '50%',
          backgroundColor: wsColor,
        }}
        title={`WebSocket: ${wsStatus}`}
      />
    </div>
  );
}
