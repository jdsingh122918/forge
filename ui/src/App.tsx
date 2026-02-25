import { useState, useEffect, useCallback, useRef } from 'react';
import type { Project, IssueColumn } from './types';
import { useBoard } from './hooks/useBoard';
import { api } from './api/client';
import { Header } from './components/Header';
import { Board } from './components/Board';
import { IssueDetail } from './components/IssueDetail';
import { NewIssueForm } from './components/NewIssueForm';
import { ProjectSetup } from './components/ProjectSetup';

function GitHubConnectDialog({ onConnected, onClose }: { onConnected: () => void; onClose: () => void }) {
  const [token, setToken] = useState('');
  const [connecting, setConnecting] = useState(false);
  const [error, setError] = useState('');
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => { inputRef.current?.focus(); }, []);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!token.trim()) return;
    setError('');
    setConnecting(true);
    try {
      await api.githubConnectToken(token.trim());
      onConnected();
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to connect');
    } finally {
      setConnecting(false);
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40" onClick={onClose}>
      <div className="bg-white rounded-lg shadow-xl w-full max-w-sm p-6" onClick={(e) => e.stopPropagation()}>
        <h3 className="text-sm font-semibold text-gray-900 mb-1">Connect GitHub</h3>
        <p className="text-xs text-gray-500 mb-4">A personal access token is required to sync issues.</p>
        {error && <p className="text-sm text-red-600 bg-red-50 rounded-md px-3 py-2 mb-3">{error}</p>}
        <form onSubmit={handleSubmit} className="space-y-3">
          <input
            ref={inputRef}
            type="password"
            placeholder="ghp_xxxxxxxxxxxxxxxxxxxx"
            value={token}
            onChange={(e) => setToken(e.target.value)}
            className="w-full px-3 py-2 border border-gray-300 rounded-md text-sm font-mono focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent"
          />
          <p className="text-xs text-gray-400">
            Create a token at{' '}
            <a href="https://github.com/settings/tokens/new?scopes=repo&description=Forge+Factory" target="_blank" rel="noopener noreferrer" className="text-blue-500 hover:underline">
              github.com/settings/tokens
            </a>
            {' '}with <code className="text-xs bg-gray-100 px-1 rounded">repo</code> scope.
          </p>
          <div className="flex gap-2">
            <button type="button" onClick={onClose} className="flex-1 px-3 py-2 text-sm text-gray-600 border border-gray-300 rounded-md hover:bg-gray-50 transition-colors">
              Cancel
            </button>
            <button type="submit" disabled={!token.trim() || connecting} className="flex-1 px-3 py-2 text-sm font-medium text-white bg-gray-900 rounded-md hover:bg-gray-800 disabled:opacity-50 transition-colors">
              {connecting ? 'Connecting...' : 'Connect & Sync'}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}

function App() {
  const [projects, setProjects] = useState<Project[]>([]);
  const [selectedProject, setSelectedProject] = useState<Project | null>(null);
  const [selectedIssueId, setSelectedIssueId] = useState<number | null>(null);
  const [showNewIssue, setShowNewIssue] = useState(false);
  const [projectsLoading, setProjectsLoading] = useState(true);
  const [syncing, setSyncing] = useState(false);
  const [showGithubConnect, setShowGithubConnect] = useState(false);

  const { board, loading, error, wsStatus, agentTeams, agentEvents, moveIssue, createIssue, deleteIssue, triggerPipeline, refresh } =
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

  const doSync = useCallback(async () => {
    if (!selectedProject) return;
    setSyncing(true);
    try {
      await api.syncGithub(selectedProject.id);
      refresh();
      const ps = await api.listProjects();
      setProjects(ps);
      const updated = ps.find((p) => p.id === selectedProject.id);
      if (updated) setSelectedProject(updated);
    } catch (e) {
      const msg = e instanceof Error ? e.message : 'Sync failed';
      console.error('Sync failed:', msg);
      alert(`GitHub sync failed: ${msg}`);
    } finally {
      setSyncing(false);
    }
  }, [selectedProject, refresh]);

  const handleSyncGithub = useCallback(async () => {
    if (!selectedProject) return;
    try {
      const status = await api.githubStatus();
      if (!status.connected) {
        setShowGithubConnect(true);
        return;
      }
      await doSync();
    } catch {
      setShowGithubConnect(true);
    }
  }, [selectedProject, doSync]);

  const handleGithubConnected = useCallback(async () => {
    setShowGithubConnect(false);
    await doSync();
  }, [doSync]);

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
            agentTeams={agentTeams}
            agentEvents={agentEvents}
            onMoveIssue={handleMoveIssue}
            onIssueClick={setSelectedIssueId}
            onTriggerPipeline={handleTriggerPipeline}
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

      {showGithubConnect && (
        <GitHubConnectDialog
          onConnected={handleGithubConnected}
          onClose={() => setShowGithubConnect(false)}
        />
      )}
    </div>
  );
}

export default App;
