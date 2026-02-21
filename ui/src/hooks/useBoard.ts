import { useState, useEffect, useCallback } from 'react';
import type { BoardView, IssueColumn } from '../types';
import { api } from '../api/client';
import { useWebSocket } from './useWebSocket';

export function useBoard(projectId: number | null) {
  const [board, setBoard] = useState<BoardView | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const wsUrl = `ws://${window.location.host}/ws`;
  const { lastMessage, status: wsStatus } = useWebSocket(wsUrl);

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

  // Apply WebSocket updates
  useEffect(() => {
    if (!lastMessage || !board) return;
    // Re-fetch board on any mutation for simplicity in MVP
    fetchBoard();
  }, [lastMessage, fetchBoard]);

  const moveIssue = useCallback(async (issueId: number, column: IssueColumn, position: number) => {
    try {
      await api.moveIssue(issueId, column, position);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to move issue');
      fetchBoard(); // Rollback by re-fetching
    }
  }, [fetchBoard]);

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
