import { useCallback } from 'react';
import {
  DndContext,
  DragEndEvent,
  PointerSensor,
  useSensor,
  useSensors,
  closestCorners,
} from '@dnd-kit/core';
import type { BoardView, IssueColumn } from '../types';
import { Column } from './Column';

interface BoardProps {
  board: BoardView;
  onMoveIssue: (issueId: number, column: IssueColumn, position: number) => void;
  onIssueClick: (issueId: number) => void;
}

export function Board({ board, onMoveIssue, onIssueClick }: BoardProps) {
  const sensors = useSensors(
    useSensor(PointerSensor, {
      activationConstraint: { distance: 8 },
    })
  );

  const handleDragEnd = useCallback((event: DragEndEvent) => {
    const { active, over } = event;
    if (!over) return;

    const issueId = parseInt(active.id as string, 10);

    // Find which column the issue was dropped into
    // The `over` can be either a column (droppable) or another card (sortable)
    let targetColumn: IssueColumn | null = null;
    let targetPosition = 0;

    // Check if dropped over a column directly
    for (const col of board.columns) {
      if (col.name === over.id) {
        targetColumn = col.name;
        targetPosition = col.issues.length;
        break;
      }
      // Check if dropped over a card in this column
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
          <Column key={column.name} column={column} onIssueClick={onIssueClick} />
        ))}
      </div>
    </DndContext>
  );
}
