import { useSortable } from '@dnd-kit/sortable';
import { CSS } from '@dnd-kit/utilities';
import type { IssueWithStatus } from '../types';
import { PRIORITY_COLORS } from '../types';
import { PipelineStatus } from './PipelineStatus';
import { PlayButton } from './PlayButton';

interface IssueCardProps {
  item: IssueWithStatus;
  onClick: (issueId: number) => void;
  onTriggerPipeline?: (issueId: number) => void;
}

export function IssueCard({ item, onClick, onTriggerPipeline }: IssueCardProps) {
  const { issue, active_run } = item;

  const {
    attributes,
    listeners,
    setNodeRef,
    transform,
    transition,
    isDragging,
  } = useSortable({ id: issue.id.toString() });

  const style = {
    transform: CSS.Transform.toString(transform),
    transition,
    opacity: isDragging ? 0.5 : 1,
  };

  const isRunning = active_run?.status === 'running';
  const isCompleted = active_run?.status === 'completed';
  const showProgress = isRunning || isCompleted;
  const currentPhase = active_run?.current_phase ?? 0;
  const phaseCount = active_run?.phase_count ?? 1;
  const progressPercent = phaseCount > 0 ? (currentPhase / phaseCount) * 100 : 0;

  return (
    <div
      ref={setNodeRef}
      style={style}
      {...attributes}
      {...listeners}
      onClick={() => onClick(issue.id)}
      className={`relative bg-white rounded-lg shadow-sm border border-gray-200 cursor-grab active:cursor-grabbing hover:border-blue-300 hover:shadow transition-all overflow-hidden ${
        isDragging ? 'ring-2 ring-blue-400' : ''
      }`}
    >
      {onTriggerPipeline && (
        <PlayButton
          issueId={issue.id}
          disabled={active_run?.status === 'queued' || active_run?.status === 'running'}
          loading={active_run?.status === 'queued'}
          onTrigger={onTriggerPipeline}
        />
      )}
      <div className="p-3">
        <p className="text-sm font-medium text-gray-900 mb-2 line-clamp-2">{issue.title}</p>
        <div className="flex items-center justify-between gap-2">
          <span className={`text-xs px-1.5 py-0.5 rounded font-medium ${PRIORITY_COLORS[issue.priority]}`}>
            {issue.priority}
          </span>
          <PipelineStatus run={active_run} compact />
        </div>
        {issue.labels.length > 0 && (
          <div className="flex flex-wrap gap-1 mt-2">
            {issue.labels.map((label) => (
              <span key={label} className="text-xs bg-gray-100 text-gray-600 px-1.5 py-0.5 rounded">
                {label}
              </span>
            ))}
          </div>
        )}
      </div>
      {showProgress && (
        <div className="w-full bg-gray-100 h-1">
          <div
            className={`h-1 rounded-br transition-all duration-300 ${isCompleted ? 'bg-green-500' : 'bg-blue-500'}`}
            style={{ width: `${isCompleted ? 100 : progressPercent}%` }}
          />
        </div>
      )}
    </div>
  );
}
