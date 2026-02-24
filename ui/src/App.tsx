import { useState, useEffect, useCallback } from 'react';
import type { Project, IssueColumn } from './types';
import { useBoard } from './hooks/useBoard';
import { api } from './api/client';
import { Header } from './components/Header';
import { Board } from './components/Board';
import { IssueDetail } from './components/IssueDetail';
import { NewIssueForm } from './components/NewIssueForm';
import { ProjectSetup } from './components/ProjectSetup';

function App() {
  const [projects, setProjects] = useState<Project[]>([]);
  const [selectedProject, setSelectedProject] = useState<Project | null>(null);
  const [selectedIssueId, setSelectedIssueId] = useState<number | null>(null);
  const [showNewIssue, setShowNewIssue] = useState(false);
  const [projectsLoading, setProjectsLoading] = useState(true);
  const [syncing, setSyncing] = useState(false);

  const { board, loading, error, wsStatus, moveIssue, createIssue, deleteIssue, triggerPipeline, refresh } =
    useBoard(selectedProject?.id ?? null);

  // Load projects on mount
  useEffect(() => {
    api.listProjects().then((ps) => {
      setProjects(ps);
      if (ps.length > 0) setSelectedProject(ps[0]);
    }).catch(console.error).finally(() => setProjectsLoading(false));
  }, []);

  const handleSelectProject = useCallback((project: Project) => {
    setSelectedProject(project);
    setSelectedIssueId(null);
    setShowNewIssue(false);
  }, []);

  const handleCreateProject = useCallback(async (name: string, path: string) => {
    const project = await api.createProject(name, path);
    setProjects((prev) => [...prev, project]);
    setSelectedProject(project);
  }, []);

  const handleCloneProject = useCallback(async (repoUrl: string) => {
    const project = await api.cloneProject(repoUrl);
    setProjects((prev) => [...prev, project]);
    setSelectedProject(project);
  }, []);

  const handleDisconnect = useCallback(() => {
    setSelectedProject(null);
    setSelectedIssueId(null);
    setShowNewIssue(false);
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

  const handleSyncGithub = useCallback(async () => {
    if (!selectedProject) return;
    setSyncing(true);
    try {
      const result = await api.syncGithub(selectedProject.id);
      if (result.imported > 0) {
        refresh();
      }
    } catch (e) {
      console.error('Sync failed:', e);
    } finally {
      setSyncing(false);
    }
  }, [selectedProject, refresh]);

  return (
    <div className="h-screen flex flex-col bg-gray-100">
      <Header
        project={selectedProject}
        projects={projects}
        wsStatus={wsStatus}
        onNewIssue={() => setShowNewIssue(true)}
        onSelectProject={handleSelectProject}
        onDisconnect={handleDisconnect}
        onSyncGithub={handleSyncGithub}
        syncing={syncing}
      />

      <main className="flex-1 overflow-hidden">
        {projectsLoading && (
          <div className="flex items-center justify-center h-full">
            <p className="text-gray-500">Loading...</p>
          </div>
        )}
        {!projectsLoading && !selectedProject && (
          <ProjectSetup
            projects={projects}
            onSelect={handleSelectProject}
            onCreate={handleCreateProject}
            onClone={handleCloneProject}
          />
        )}
        {selectedProject && loading && (
          <div className="flex items-center justify-center h-full">
            <p className="text-gray-500">Loading board...</p>
          </div>
        )}
        {selectedProject && error && (
          <div className="p-6">
            <p className="text-red-500 bg-red-50 rounded-md p-4">{error}</p>
          </div>
        )}
        {selectedProject && board && (
          <Board
            board={board}
            onMoveIssue={handleMoveIssue}
            onIssueClick={setSelectedIssueId}
            backlogHeaderAction={
              !showNewIssue ? (
                <button
                  onClick={() => setShowNewIssue(true)}
                  className="text-gray-400 hover:text-blue-600 hover:bg-blue-50 rounded p-0.5 transition-colors"
                  title="New Issue"
                >
                  <svg className="h-4 w-4" viewBox="0 0 20 20" fill="currentColor">
                    <path fillRule="evenodd" d="M10 3a1 1 0 011 1v5h5a1 1 0 110 2h-5v5a1 1 0 11-2 0v-5H4a1 1 0 110-2h5V4a1 1 0 011-1z" clipRule="evenodd" />
                  </svg>
                </button>
              ) : undefined
            }
            backlogTopSlot={
              showNewIssue ? (
                <NewIssueForm
                  onSubmit={handleCreateIssue}
                  onCancel={() => setShowNewIssue(false)}
                />
              ) : undefined
            }
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
    </div>
  );
}

export default App;
