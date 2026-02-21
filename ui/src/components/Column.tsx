import { useDroppable } from '@dnd-kit/core';
import { SortableContext, verticalListSortingStrategy } from '@dnd-kit/sortable';
import type { ColumnView } from '../types';
import { IssueCard } from './IssueCard';

interface ColumnProps {
  column: ColumnView;
  onIssueClick: (issueId: number) => void;
}

const COLUMN_LABELS: Record<string, string> = {
  backlog: 'Backlog',
  ready: 'Ready',
  in_progress: 'In Progress',
  in_review: 'In Review',
  done: 'Done',
};

const COLUMN_COLORS: Record<string, string> = {
  backlog: 'border-t-gray-400',
  ready: 'border-t-blue-400',
  in_progress: 'border-t-yellow-400',
  in_review: 'border-t-purple-400',
  done: 'border-t-green-400',
};

export function Column({ column, onIssueClick }: ColumnProps) {
  const { setNodeRef, isOver } = useDroppable({ id: column.name });

  const itemIds = column.issues.map((item) => item.issue.id.toString());

  return (
    <div
      ref={setNodeRef}
      className={`flex flex-col bg-gray-50 rounded-lg border-t-4 ${COLUMN_COLORS[column.name] || 'border-t-gray-300'} ${
        isOver ? 'ring-2 ring-blue-300 bg-blue-50/30' : ''
      }`}
    >
      <div className="px-3 py-2.5 flex items-center justify-between">
        <h3 className="text-sm font-semibold text-gray-700 uppercase tracking-wider">
          {COLUMN_LABELS[column.name] || column.name}
        </h3>
        <span className="text-xs font-medium text-gray-400 bg-gray-200 rounded-full px-2 py-0.5">
          {column.issues.length}
        </span>
      </div>
      <SortableContext items={itemIds} strategy={verticalListSortingStrategy}>
        <div className="flex-1 px-2 pb-2 space-y-2 min-h-[100px]">
          {column.issues.map((item) => (
            <IssueCard key={item.issue.id} item={item} onClick={onIssueClick} />
          ))}
        </div>
      </SortableContext>
    </div>
  );
}
