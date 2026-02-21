import type { PipelineRun, PipelineStatus as PipelineStatusType } from '../types';
import { STATUS_COLORS } from '../types';

interface PipelineStatusProps {
  run: PipelineRun | null;
  compact?: boolean;
}

export function PipelineStatus({ run, compact = false }: PipelineStatusProps) {
  if (!run) return null;

  const icon = getStatusIcon(run.status);
  const color = STATUS_COLORS[run.status];

  if (compact) {
    return (
      <span className={`inline-flex items-center gap-1 text-xs ${color}`} title={`Pipeline: ${run.status}`}>
        {icon} {run.status}
      </span>
    );
  }

  return (
    <div className={`flex items-center gap-2 text-sm ${color}`}>
      {icon}
      <span className="capitalize">{run.status}</span>
      {run.status === 'running' && run.current_phase && run.phase_count && (
        <span className="text-xs text-gray-400">
          Phase {run.current_phase}/{run.phase_count}
        </span>
      )}
    </div>
  );
}

function getStatusIcon(status: PipelineStatusType): string {
  switch (status) {
    case 'queued': return '\u23F3';
    case 'running': return '\u25B6';
    case 'completed': return '\u2714';
    case 'failed': return '\u2716';
    case 'cancelled': return '\u23F9';
  }
}
