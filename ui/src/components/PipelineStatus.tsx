import type { PipelineRun, PipelineStatus as PipelineStatusType } from '../types';
import { STATUS_COLORS } from '../types';

interface PipelineStatusProps {
  run: PipelineRun | null;
  compact?: boolean;
}

function Spinner({ className = 'h-4 w-4' }: { className?: string }) {
  return (
    <svg className={`animate-spin ${className}`} viewBox="0 0 24 24" fill="none">
      <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" />
      <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z" />
    </svg>
  );
}

function StatusIcon({ status }: { status: PipelineStatusType }) {
  if (status === 'running') {
    return <Spinner className="h-3.5 w-3.5" />;
  }
  return <span>{getStaticIcon(status)}</span>;
}

function getStaticIcon(status: PipelineStatusType): string {
  switch (status) {
    case 'queued': return '\u23F3';
    case 'running': return '\u25B6';
    case 'completed': return '\u2714';
    case 'failed': return '\u2716';
    case 'cancelled': return '\u23F9';
  }
}

export function PipelineStatus({ run, compact = false }: PipelineStatusProps) {
  if (!run) return null;

  const color = STATUS_COLORS[run.status];

  if (compact) {
    return (
      <span className={`inline-flex items-center gap-1 text-xs ${color}`} title={`Pipeline: ${run.status}`}>
        <StatusIcon status={run.status} /> {run.status}
      </span>
    );
  }

  return (
    <div className={`flex items-center gap-2 text-sm ${color}`}>
      <StatusIcon status={run.status} />
      <span className="capitalize">{run.status}</span>
      {run.status === 'running' && run.current_phase && run.phase_count && (
        <span className="text-xs text-gray-400">
          Phase {run.current_phase}/{run.phase_count}
        </span>
      )}
    </div>
  );
}
