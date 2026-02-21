import { useState, useEffect, useCallback } from 'react';
import type { Project, IssueColumn } from './types';
import { useBoard } from './hooks/useBoard';
import { api } from './api/client';
import { Header } from './components/Header';
import { Board } from './components/Board';
import { IssueDetail } from './components/IssueDetail';
import { NewIssueForm } from './components/NewIssueForm';

function App() {
  const [, setProjects] = useState<Project[]>([]);
  const [selectedProject, setSelectedProject] = useState<Project | null>(null);
  const [selectedIssueId, setSelectedIssueId] = useState<number | null>(null);
  const [showNewIssue, setShowNewIssue] = useState(false);

  const { board, loading, error, wsStatus, moveIssue, createIssue, deleteIssue, triggerPipeline, refresh } =
    useBoard(selectedProject?.id ?? null);

  // Load projects on mount
  useEffect(() => {
    api.listProjects().then((ps) => {
      setProjects(ps);
      if (ps.length > 0) setSelectedProject(ps[0]);
    }).catch(console.error);
  }, []);

  const handleMoveIssue = useCallback(
    (issueId: number, column: IssueColumn, position: number) => {
      moveIssue(issueId, column, position);
    },
    [moveIssue]
  );

  const handleCreateIssue = useCallback(
    async (title: string, description: string) => {
      await createIssue(title, description);
      setShowNewIssue(false);
      refresh();
    },
    [createIssue, refresh]
  );

  const handleDeleteIssue = useCallback(
    async (issueId: number) => {
      await deleteIssue(issueId);
      setSelectedIssueId(null);
      refresh();
    },
    [deleteIssue, refresh]
  );

  const handleTriggerPipeline = useCallback(
    async (issueId: number) => {
      await triggerPipeline(issueId);
      refresh();
    },
    [triggerPipeline, refresh]
  );

  return (
    <div className="h-screen flex flex-col bg-gray-100">
      <Header
        projectName={selectedProject?.name ?? null}
        wsStatus={wsStatus}
        onNewIssue={() => setShowNewIssue(true)}
      />

      <main className="flex-1 overflow-hidden">
        {loading && (
          <div className="flex items-center justify-center h-full">
            <p className="text-gray-500">Loading board...</p>
          </div>
        )}
        {error && (
          <div className="p-6">
            <p className="text-red-500 bg-red-50 rounded-md p-4">{error}</p>
          </div>
        )}
        {!board && !loading && !error && (
          <div className="flex items-center justify-center h-full">
            <div className="text-center">
              <h2 className="text-lg font-medium text-gray-700">No project selected</h2>
              <p className="text-gray-500 mt-2">Select or create a project to get started.</p>
            </div>
          </div>
        )}
        {board && (
          <Board
            board={board}
            onMoveIssue={handleMoveIssue}
            onIssueClick={setSelectedIssueId}
          />
        )}
      </main>

      {selectedIssueId && (
        <IssueDetail
          issueId={selectedIssueId}
          onClose={() => setSelectedIssueId(null)}
          onTriggerPipeline={handleTriggerPipeline}
          onDelete={handleDeleteIssue}
        />
      )}

      {showNewIssue && (
        <NewIssueForm
          onSubmit={handleCreateIssue}
          onCancel={() => setShowNewIssue(false)}
        />
      )}
    </div>
  );
}

export default App;
