import type { ReactNode } from 'react';
import { useDroppable } from '@dnd-kit/core';
import { SortableContext, verticalListSortingStrategy } from '@dnd-kit/sortable';
import type { ColumnView, AgentTeamDetail, AgentEvent } from '../types';
import { IssueCard } from './IssueCard';
import { AgentTeamPanel } from './AgentTeamPanel';
import { VerificationPanel } from './VerificationPanel';

interface ColumnProps {
  column: ColumnView;
  onIssueClick: (issueId: number) => void;
  onTriggerPipeline?: (issueId: number) => void;
  agentTeams?: Map<number, AgentTeamDetail>;
  agentEvents?: Map<number, AgentEvent[]>;
  headerAction?: ReactNode;
  topSlot?: ReactNode;
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

function formatElapsedFromStr(startedAt: string | null | undefined): string {
  if (!startedAt) return '--';
  const seconds = Math.floor((Date.now() - new Date(startedAt).getTime()) / 1000);
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  const secs = seconds % 60;
  return `${minutes}m ${secs}s`;
}

export function Column({ column, onIssueClick, onTriggerPipeline, agentTeams, agentEvents, headerAction, topSlot }: ColumnProps) {
  const { setNodeRef, isOver } = useDroppable({ id: column.name });

  const itemIds = column.issues.map((item) => item.issue.id.toString());

  return (
    <div
      ref={setNodeRef}
      className={`flex flex-col min-h-0 bg-gray-50 rounded-lg border-t-4 ${COLUMN_COLORS[column.name] || 'border-t-gray-300'} ${
        isOver ? 'ring-2 ring-blue-300 bg-blue-50/30' : ''
      }`}
    >
      <div className="px-3 py-2.5 flex items-center justify-between">
        <div className="flex items-center gap-2">
          <h3 className="text-sm font-semibold text-gray-700 uppercase tracking-wider">
            {COLUMN_LABELS[column.name] || column.name}
          </h3>
          <span className="text-xs font-medium text-gray-400 bg-gray-200 rounded-full px-2 py-0.5">
            {column.issues.length}
          </span>
        </div>
        {headerAction}
      </div>
      <SortableContext items={itemIds} strategy={verticalListSortingStrategy}>
        <div className="flex-1 overflow-y-auto px-2 pb-2 space-y-2 min-h-[100px]">
          {topSlot}
          {column.issues.map((item) => {
            const teamDetail = item.active_run && agentTeams
              ? agentTeams.get(item.active_run.id)
              : undefined;

            // In Progress column: show AgentTeamPanel if team data exists
            if (column.name === 'in_progress' && teamDetail && agentEvents) {
              return (
                <div key={item.issue.id}>
                  <div className="text-sm font-medium text-gray-900 mb-2 px-1 cursor-pointer hover:text-blue-600" onClick={() => onIssueClick(item.issue.id)}>
                    {item.issue.title}
                  </div>
                  <AgentTeamPanel
                    teamDetail={teamDetail}
                    agentEvents={agentEvents}
                    elapsedTime={formatElapsedFromStr(item.active_run?.started_at)}
                  />
                </div>
              );
            }

            // In Review column: show VerificationPanel below card if data exists
            if (column.name === 'in_review' && item.active_run && agentEvents) {
              const verificationEvents = [...agentEvents.values()].flat().filter(e =>
                e.metadata?.verification_type
              );
              return (
                <div key={item.issue.id}>
                  <IssueCard item={item} onClick={onIssueClick} onTriggerPipeline={onTriggerPipeline} />
                  {verificationEvents.length > 0 && (
                    <VerificationPanel
                      run={item.active_run}
                      verificationEvents={verificationEvents}
                    />
                  )}
                </div>
              );
            }

            // Default: standard IssueCard
            return <IssueCard key={item.issue.id} item={item} onClick={onIssueClick} onTriggerPipeline={onTriggerPipeline} />;
          })}
        </div>
      </SortableContext>
    </div>
  );
}
