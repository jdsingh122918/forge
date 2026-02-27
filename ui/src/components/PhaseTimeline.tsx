import type { PipelinePhase } from '../types';

interface PhaseTimelineProps {
  phases: PipelinePhase[];
}

function StatusIcon({ status }: { status: string }) {
  switch (status) {
    case 'completed':
      return <span className="text-green-500">{'\u2714'}</span>;
    case 'failed':
      return <span className="text-red-500">{'\u2716'}</span>;
    case 'running':
      return (
        <svg className="animate-spin h-3.5 w-3.5 text-blue-500" viewBox="0 0 24 24" fill="none">
          <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" />
          <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z" />
        </svg>
      );
    default:
      return <span className="text-gray-300">{'\u25CB'}</span>;
  }
}

function formatDuration(startedAt: string | null, completedAt: string | null): string | null {
  if (!startedAt || !completedAt) return null;
  const start = new Date(startedAt).getTime();
  const end = new Date(completedAt).getTime();
  const secs = Math.round((end - start) / 1000);
  if (secs < 60) return `${secs}s`;
  const mins = Math.floor(secs / 60);
  const remainSecs = secs % 60;
  return `${mins}m ${remainSecs}s`;
}

export function PhaseTimeline({ phases }: PhaseTimelineProps) {
  if (phases.length === 0) return null;

  return (
    <div className="space-y-1">
      {phases.map((phase) => {
        const duration = formatDuration(phase.started_at, phase.completed_at);
        const iterInfo = phase.iteration != null && phase.budget != null
          ? `iter ${phase.iteration}/${phase.budget}`
          : null;

        return (
          <div
            key={phase.id}
            className={`flex items-center gap-2 text-xs py-1 px-2 rounded ${
              phase.status === 'running' ? 'bg-blue-50' : ''
            }`}
          >
            <StatusIcon status={phase.status} />
            <span className="text-gray-500 font-mono w-6 text-right">{phase.phase_number}</span>
            <span className={`flex-1 truncate ${
              phase.status === 'running' ? 'text-blue-700 font-medium' : 'text-gray-700'
            }`}>
              {phase.phase_name}
            </span>
            {phase.review_status === 'reviewing' && (
              <span className="text-yellow-600 shrink-0">Reviewing...</span>
            )}
            {phase.review_status === 'passed' && (
              <span className="text-green-600 shrink-0">Review passed</span>
            )}
            {phase.review_status === 'failed' && (
              <span className="text-amber-600 shrink-0">{phase.review_findings} finding{phase.review_findings !== 1 ? 's' : ''}</span>
            )}
            <span className="text-gray-400 shrink-0">
              {phase.status === 'running' && iterInfo}
              {duration && ` ${duration}`}
            </span>
          </div>
        );
      })}
    </div>
  );
}
