import { useState, useEffect, useCallback, useRef } from 'react';
import type { BoardView, IssueColumn, WsMessage, IssueWithStatus } from '../types';
import { api } from '../api/client';
import { useWebSocket } from './useWebSocket';

export function useBoard(projectId: number | null) {
  const [board, setBoard] = useState<BoardView | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const prevBoardRef = useRef<BoardView | null>(null);

  const wsUrl = `ws://${window.location.host}/ws`;
  const { lastMessage, connectionStatus: wsStatus } = useWebSocket(wsUrl);

  // Fetch initial board
  const fetchBoard = useCallback(async () => {
    if (!projectId) return;
    setLoading(true);
    setError(null);
    try {
      const data = await api.getBoard(projectId);
      setBoard(data);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to load board');
    } finally {
      setLoading(false);
    }
  }, [projectId]);

  useEffect(() => {
    fetchBoard();
  }, [fetchBoard]);

  // Apply WebSocket updates incrementally
  useEffect(() => {
    if (!lastMessage || !board) return;
    const msg = lastMessage as WsMessage;

    setBoard(prev => {
      if (!prev) return prev;

      switch (msg.type) {
        case 'IssueCreated': {
          const { issue } = msg.data;
          const newItem: IssueWithStatus = { issue, active_run: null };
          return {
            ...prev,
            columns: prev.columns.map(col =>
              col.name === issue.column
                ? { ...col, issues: [...col.issues, newItem] }
                : col
            ),
          };
        }
        case 'IssueUpdated': {
          const { issue } = msg.data;
          return {
            ...prev,
            columns: prev.columns.map(col => ({
              ...col,
              issues: col.issues.map(item =>
                item.issue.id === issue.id
                  ? { ...item, issue }
                  : item
              ),
            })),
          };
        }
        case 'IssueMoved': {
          const { issue_id, from_column, to_column, position } = msg.data;
          // Find the issue in the source column
          let movedItem: IssueWithStatus | undefined;
          const withoutSource = prev.columns.map(col => {
            if (col.name === from_column) {
              const idx = col.issues.findIndex(i => i.issue.id === issue_id);
              if (idx >= 0) {
                movedItem = col.issues[idx];
                return { ...col, issues: col.issues.filter((_, i) => i !== idx) };
              }
            }
            return col;
          });
          if (!movedItem) return prev; // Not found, re-fetch as fallback
          // Update the issue's column
          movedItem = { ...movedItem, issue: { ...movedItem.issue, column: to_column as IssueColumn } };
          // Insert at position in target column
          return {
            ...prev,
            columns: withoutSource.map(col => {
              if (col.name === to_column) {
                const issues = [...col.issues];
                issues.splice(Math.min(position, issues.length), 0, movedItem!);
                return { ...col, issues };
              }
              return col;
            }),
          };
        }
        case 'IssueDeleted': {
          const { issue_id } = msg.data;
          return {
            ...prev,
            columns: prev.columns.map(col => ({
              ...col,
              issues: col.issues.filter(item => item.issue.id !== issue_id),
            })),
          };
        }
        case 'PipelineStarted': {
          const { run } = msg.data;
          return {
            ...prev,
            columns: prev.columns.map(col => ({
              ...col,
              issues: col.issues.map(item =>
                item.issue.id === run.issue_id
                  ? { ...item, active_run: run }
                  : item
              ),
            })),
          };
        }
        case 'PipelineProgress': {
          const { run_id, phase, iteration } = msg.data;
          return {
            ...prev,
            columns: prev.columns.map(col => ({
              ...col,
              issues: col.issues.map(item =>
                item.active_run?.id === run_id
                  ? { ...item, active_run: { ...item.active_run, current_phase: phase, iteration } }
                  : item
              ),
            })),
          };
        }
        case 'PipelineCompleted':
        case 'PipelineFailed': {
          const { run } = msg.data;
          return {
            ...prev,
            columns: prev.columns.map(col => ({
              ...col,
              issues: col.issues.map(item =>
                item.issue.id === run.issue_id
                  ? { ...item, active_run: run }
                  : item
              ),
            })),
          };
        }
        case 'PipelineBranchCreated': {
          const { run_id, branch_name } = msg.data;
          return {
            ...prev,
            columns: prev.columns.map(col => ({
              ...col,
              issues: col.issues.map(item =>
                item.active_run?.id === run_id
                  ? { ...item, active_run: { ...item.active_run, branch_name } }
                  : item
              ),
            })),
          };
        }
        case 'PipelinePrCreated': {
          const { run_id, pr_url } = msg.data;
          return {
            ...prev,
            columns: prev.columns.map(col => ({
              ...col,
              issues: col.issues.map(item =>
                item.active_run?.id === run_id
                  ? { ...item, active_run: { ...item.active_run, pr_url } }
                  : item
              ),
            })),
          };
        }
        case 'TeamCreated':
        case 'WaveStarted':
        case 'WaveCompleted':
        case 'AgentTaskStarted':
        case 'AgentTaskCompleted':
        case 'AgentTaskFailed':
        case 'AgentThinking':
        case 'AgentAction':
        case 'AgentOutput':
        case 'AgentSignal':
        case 'MergeStarted':
        case 'MergeCompleted':
        case 'MergeConflict':
        case 'VerificationResult':
          return prev; // Handled outside setBoard callback
        default:
          // Unknown message type — fall back to full re-fetch
          fetchBoard();
          return prev;
      }
    });
  }, [lastMessage, fetchBoard]);

  // Optimistic move: update local state immediately, rollback on error
  const moveIssue = useCallback(async (issueId: number, column: IssueColumn, position: number) => {
    // Save current board for rollback
    prevBoardRef.current = board;

    // Optimistic local update
    setBoard(prev => {
      if (!prev) return prev;
      let movedItem: IssueWithStatus | undefined;
      const withoutSource = prev.columns.map(col => {
        const idx = col.issues.findIndex(i => i.issue.id === issueId);
        if (idx >= 0) {
          movedItem = col.issues[idx];
          return { ...col, issues: col.issues.filter((_, i) => i !== idx) };
        }
        return col;
      });
      if (!movedItem) return prev;
      movedItem = { ...movedItem, issue: { ...movedItem.issue, column } };
      return {
        ...prev,
        columns: withoutSource.map(col => {
          if (col.name === column) {
            const issues = [...col.issues];
            issues.splice(Math.min(position, issues.length), 0, movedItem!);
            return { ...col, issues };
          }
          return col;
        }),
      };
    });

    try {
      await api.moveIssue(issueId, column, position);
    } catch (e) {
      // Rollback on failure
      setBoard(prevBoardRef.current);
      setError(e instanceof Error ? e.message : 'Failed to move issue');
    }
  }, [board]);

  const createIssue = useCallback(async (title: string, description: string) => {
    if (!projectId) return;
    try {
      await api.createIssue(projectId, title, description);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to create issue');
    }
  }, [projectId]);

  const deleteIssue = useCallback(async (issueId: number) => {
    try {
      await api.deleteIssue(issueId);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to delete issue');
    }
  }, []);

  const triggerPipeline = useCallback(async (issueId: number) => {
    try {
      await api.triggerPipeline(issueId);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to trigger pipeline');
    }
  }, []);

  return { board, loading, error, wsStatus, moveIssue, createIssue, deleteIssue, triggerPipeline, refresh: fetchBoard };
}
