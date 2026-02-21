import { useSortable } from '@dnd-kit/sortable';
import { CSS } from '@dnd-kit/utilities';
import type { IssueWithStatus } from '../types';
import { PRIORITY_COLORS } from '../types';
import { PipelineStatus } from './PipelineStatus';

interface IssueCardProps {
  item: IssueWithStatus;
  onClick: (issueId: number) => void;
}

export function IssueCard({ item, onClick }: IssueCardProps) {
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

  return (
    <div
      ref={setNodeRef}
      style={style}
      {...attributes}
      {...listeners}
      onClick={() => onClick(issue.id)}
      className={`bg-white rounded-lg p-3 shadow-sm border border-gray-200 cursor-grab active:cursor-grabbing hover:border-blue-300 hover:shadow transition-all ${
        isDragging ? 'ring-2 ring-blue-400' : ''
      }`}
    >
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
  );
}
