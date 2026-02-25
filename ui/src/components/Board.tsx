import { useCallback } from 'react';
import type { ReactNode } from 'react';
import {
  DndContext,
  PointerSensor,
  useSensor,
  useSensors,
  closestCorners,
} from '@dnd-kit/core';
import type { DragEndEvent } from '@dnd-kit/core';
import type { BoardView, IssueColumn, AgentTeamDetail, AgentEvent } from '../types';
import { Column } from './Column';

interface BoardProps {
  board: BoardView;
  agentTeams?: Map<number, AgentTeamDetail>;
  agentEvents?: Map<number, AgentEvent[]>;
  onMoveIssue: (issueId: number, column: IssueColumn, position: number) => void;
  onIssueClick: (issueId: number) => void;
  onTriggerPipeline?: (issueId: number) => void;
  backlogHeaderAction?: ReactNode;
  backlogTopSlot?: ReactNode;
}

export function Board({ board, agentTeams, agentEvents, onMoveIssue, onIssueClick, onTriggerPipeline, backlogHeaderAction, backlogTopSlot }: BoardProps) {
  const sensors = useSensors(
    useSensor(PointerSensor, {
      activationConstraint: { distance: 8 },
    })
  );

  const handleDragEnd = useCallback((event: DragEndEvent) => {
    const { active, over } = event;
    if (!over) return;

    const issueId = parseInt(active.id as string, 10);

    let targetColumn: IssueColumn | null = null;
    let targetPosition = 0;

    for (const col of board.columns) {
      if (col.name === over.id) {
        targetColumn = col.name;
        targetPosition = col.issues.length;
        break;
      }
      const cardIndex = col.issues.findIndex(
        (item) => item.issue.id.toString() === over.id
      );
      if (cardIndex >= 0) {
        targetColumn = col.name;
        targetPosition = cardIndex;
        break;
      }
    }

    if (targetColumn) {
      onMoveIssue(issueId, targetColumn, targetPosition);
    }
  }, [board, onMoveIssue]);

  return (
    <DndContext
      sensors={sensors}
      collisionDetection={closestCorners}
      onDragEnd={handleDragEnd}
    >
      <div className="grid grid-cols-5 gap-4 h-[calc(100vh-64px)] p-6">
        {board.columns.map((column) => (
          <Column
            key={column.name}
            column={column}
            onIssueClick={onIssueClick}
            onTriggerPipeline={onTriggerPipeline}
            agentTeams={agentTeams}
            agentEvents={agentEvents}
            headerAction={column.name === 'backlog' ? backlogHeaderAction : undefined}
            topSlot={column.name === 'backlog' ? backlogTopSlot : undefined}
          />
        ))}
      </div>
    </DndContext>
  );
}
