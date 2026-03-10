import { useState, useCallback, useMemo } from 'react';
import type { ViewMode, Project } from './types';
import useMissionControl from './hooks/useMissionControl';
import { WebSocketProvider } from './contexts/WebSocketContext';
import StatusBar from './components/StatusBar';
import ProjectSidebar from './components/ProjectSidebar';
import AgentRunCard from './components/AgentRunCard';
import EventLog from './components/EventLog';
import FloatingActionButton from './components/FloatingActionButton';
import NewIssueModal from './components/NewIssueModal';
import ConfirmDialog from './components/ConfirmDialog';
import { ProjectSetup } from './components/ProjectSetup';
import Analytics from './components/Analytics';
import Agents from './components/Agents';
import { api } from './api/client';

/**
 * Mission Control dashboard — the main application shell.
 * Aggregates all projects and agent runs into a unified monitoring view.
 * Layout: StatusBar (top) -> flex row [ProjectSidebar | Agent Grid] -> EventLog (bottom).
 */
function MissionControl() {
  const {
    projects,
    agentRunCards,
    idleIssueCards,
    statusCounts,
    eventLog,
    phases,
    agentTeams,
    agentEvents,
    pipelineEvents,
    pipelineOutputEvents,
    pipelineFileChanges,
    loading,
    selectedProjectId,
    setSelectedProjectId,
    triggerPipeline,
    cancelPipeline,
    createIssue,
    createProject,
    cloneProject,
    deleteProject,
    issuesByProject: _issuesByProject,
  } = useMissionControl();

  const [viewMode, setViewMode] = useState<ViewMode>('grid');
  const [showNewIssueModal, setShowNewIssueModal] = useState(false);
  const [newIssueProjectId, setNewIssueProjectId] = useState<number | null>(null);
  const [showProjectSetup, setShowProjectSetup] = useState(false);
  const [deleteConfirm, setDeleteConfirm] = useState<{ id: number; name: string } | null>(null);

  /** Compute run counts by project for the sidebar */
  const runsByProject = useMemo(() => {
    const map = new Map<number, { running: number; total: number }>();
    for (const card of agentRunCards) {
      const pid = card.project.id;
      const existing = map.get(pid) ?? { running: 0, total: 0 };
      existing.total++;
      if (card.run.status === 'running') existing.running++;
      map.set(pid, existing);
    }
    return map;
  }, [agentRunCards]);

  /** Projects filtered by the current sidebar selection */
  const displayedProjects = useMemo(() => {
    if (selectedProjectId === null) return projects;
    return projects.filter(p => p.id === selectedProjectId);
  }, [projects, selectedProjectId]);

  const handleNewIssue = useCallback(() => {
    setNewIssueProjectId(null);
    setShowNewIssueModal(true);
  }, []);

  const handleNewProject = useCallback(() => {
    setShowProjectSetup(true);
  }, []);

  const handleSyncGithub = useCallback(async () => {
    const project = projects.find(p =>
      selectedProjectId === null ? true : p.id === selectedProjectId
    );
    if (project) {
      try {
        await api.syncGithub(project.id);
      } catch (err) {
        console.error('Sync failed:', err);
      }
    }
  }, [projects, selectedProjectId]);

  const handleIssueSubmit = useCallback(async (projectId: number, title: string, description: string) => {
    const issue = await createIssue(projectId, title, description);
    await triggerPipeline(issue.id);
  }, [createIssue, triggerPipeline]);

  const handleProjectSelect = useCallback((project: Project) => {
    setSelectedProjectId(project.id);
    setShowProjectSetup(false);
  }, [setSelectedProjectId]);

  const handleProjectCreate = useCallback((name: string, path: string) => {
    createProject(name, path).then(() => {
      setShowProjectSetup(false);
    }).catch(console.error);
  }, [createProject]);

  const handleCloneProject = useCallback(async (repoUrl: string) => {
    const project = await cloneProject(repoUrl);
    setShowProjectSetup(false);
    setSelectedProjectId(project.id);
  }, [cloneProject, setSelectedProjectId]);

  const handleDeleteProject = useCallback(async (projectId: number) => {
    await deleteProject(projectId);
    if (selectedProjectId === projectId) {
      setSelectedProjectId(null);
    }
  }, [deleteProject, selectedProjectId, setSelectedProjectId]);

  // Loading state
  if (loading) {
    return (
      <div style={{
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        height: '100vh',
        backgroundColor: 'var(--color-bg-primary)',
        color: 'var(--color-text-primary)',
        gap: '12px',
        fontSize: '14px',
      }}>
        <span className="pulse-dot" style={{
          width: '8px',
          height: '8px',
          borderRadius: '50%',
          backgroundColor: 'var(--color-success)',
        }} />
        Initializing Mission Control...
      </div>
    );
  }

  // No projects — show ProjectSetup full-screen
  if (projects.length === 0) {
    return (
      <div style={{
        height: '100vh',
        backgroundColor: 'var(--color-bg-primary)',
      }}>
        <ProjectSetup
          projects={[]}
          onSelect={handleProjectSelect}
          onCreate={handleProjectCreate}
          onClone={handleCloneProject}
        />
      </div>
    );
  }

  return (
    <div style={{
      display: 'flex',
      flexDirection: 'column',
      height: '100vh',
      backgroundColor: 'var(--color-bg-primary)',
      color: 'var(--color-text-primary)',
    }}>
      {/* Top bar */}
      <StatusBar
        agentCounts={{
          running: statusCounts.running,
          queued: statusCounts.queued,
          completed: statusCounts.completed,
          failed: statusCounts.failed,
        }}
        projectCount={projects.length}
        viewMode={viewMode}
        onViewModeChange={setViewMode}
      />

      {/* Main content: sidebar + agent grid */}
      <div style={{ flex: 1, display: 'flex', overflow: 'hidden' }}>
        <ProjectSidebar
          projects={projects}
          selectedProjectId={selectedProjectId}
          onSelectProject={setSelectedProjectId}
          onDeleteProject={(id, name) => setDeleteConfirm({ id, name })}
          runsByProject={runsByProject}
        />

        {/* Main content area */}
        {viewMode === 'analytics' ? (
          <div style={{ flex: 1, overflow: 'hidden' }}>
            <Analytics />
          </div>
        ) : viewMode === 'agents' ? (
          <div style={{ flex: 1, overflow: 'hidden' }}>
            <Agents />
          </div>
        ) : (
          <div style={{
            flex: 1,
            overflowY: 'auto',
            padding: '16px',
          }}>
            <div style={{
              display: viewMode === 'grid' ? 'grid' : 'flex',
              gridTemplateColumns: viewMode === 'grid' ? 'repeat(auto-fill, minmax(400px, 1fr))' : undefined,
              flexDirection: viewMode === 'list' ? 'column' : undefined,
              gap: '12px',
            }}>
              {/* Active pipeline run cards */}
              {agentRunCards.map(card => (
                <AgentRunCard
                  key={`run-${card.run.id}`}
                  card={card}
                  phases={phases.get(card.run.id)}
                  agentTeam={agentTeams.get(card.run.id)}
                  agentEvents={agentEvents}
                  pipelineEvents={pipelineEvents.get(card.run.id)}
                  pipelineOutputEvents={pipelineOutputEvents.get(card.run.id)}
                  pipelineFileChanges={pipelineFileChanges.get(card.run.id)}
                  onCancel={cancelPipeline}
                  viewMode={viewMode}
                />
              ))}

              {/* Idle issue cards — issues without active pipeline runs */}
              {idleIssueCards.map(({ issue, project }) => (
                <div
                  key={`idle-${issue.id}`}
                  style={{
                    backgroundColor: 'var(--color-bg-card)',
                    border: '1px solid var(--color-border)',
                    borderLeft: '3px solid var(--color-text-secondary)',
                    transition: 'background-color 0.15s',
                  }}
                  onMouseEnter={e => (e.currentTarget.style.backgroundColor = 'var(--color-bg-card-hover)')}
                  onMouseLeave={e => (e.currentTarget.style.backgroundColor = 'var(--color-bg-card)')}
                >
                  <div style={{
                    display: 'flex',
                    alignItems: 'center',
                    padding: '12px',
                    gap: '12px',
                  }}>
                    {/* Idle dot */}
                    <span style={{
                      width: '8px',
                      height: '8px',
                      borderRadius: '50%',
                      backgroundColor: 'var(--color-text-secondary)',
                      flexShrink: 0,
                    }} />

                    {/* Project badge + issue title */}
                    <div style={{ flex: 1, minWidth: 0 }}>
                      <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
                        <span style={{
                          fontSize: '10px',
                          padding: '1px 6px',
                          backgroundColor: 'var(--color-border)',
                          color: 'var(--color-text-secondary)',
                          textTransform: 'uppercase',
                          letterSpacing: '0.5px',
                          flexShrink: 0,
                        }}>
                          {project.name}
                        </span>
                        <span style={{
                          overflow: 'hidden',
                          textOverflow: 'ellipsis',
                          whiteSpace: 'nowrap',
                          fontSize: '13px',
                        }}>
                          {issue.title}
                        </span>
                      </div>
                    </div>

                    {/* Status label */}
                    <span style={{
                      fontSize: '11px',
                      color: 'var(--color-text-secondary)',
                      fontWeight: 600,
                      flexShrink: 0,
                    }}>
                      IDLE
                    </span>

                    {/* Run button */}
                    <button
                      onClick={() => triggerPipeline(issue.id)}
                      style={{
                        padding: '4px 12px',
                        fontSize: '11px',
                        fontFamily: 'inherit',
                        background: 'transparent',
                        border: '1px solid var(--color-success)',
                        color: 'var(--color-success)',
                        cursor: 'pointer',
                        flexShrink: 0,
                      }}
                      onMouseEnter={e => {
                        e.currentTarget.style.backgroundColor = 'var(--color-success)';
                        e.currentTarget.style.color = '#000';
                      }}
                      onMouseLeave={e => {
                        e.currentTarget.style.backgroundColor = 'transparent';
                        e.currentTarget.style.color = 'var(--color-success)';
                      }}
                    >
                      Run
                    </button>
                  </div>
                </div>
              ))}

              {/* Empty state — no issues at all */}
              {agentRunCards.length === 0 && idleIssueCards.length === 0 && displayedProjects.map(project => (
                <div
                  key={`empty-${project.id}`}
                  style={{
                    backgroundColor: 'var(--color-bg-card)',
                    border: '1px solid var(--color-border)',
                    borderLeft: '3px solid var(--color-text-secondary)',
                  }}
                >
                  <div style={{
                    display: 'flex',
                    alignItems: 'center',
                    padding: '12px',
                    gap: '12px',
                  }}>
                    <span style={{
                      width: '8px',
                      height: '8px',
                      borderRadius: '50%',
                      backgroundColor: 'var(--color-text-secondary)',
                      flexShrink: 0,
                    }} />
                    <div style={{ flex: 1, minWidth: 0 }}>
                      <div style={{
                        fontSize: '13px',
                        fontWeight: 600,
                      }}>
                        {project.name}
                      </div>
                      <div style={{
                        fontSize: '11px',
                        color: 'var(--color-text-secondary)',
                        marginTop: '2px',
                      }}>
                        No issues yet
                      </div>
                    </div>
                    <button
                      onClick={() => {
                        setNewIssueProjectId(project.id);
                        setShowNewIssueModal(true);
                      }}
                      style={{
                        padding: '4px 10px',
                        fontSize: '11px',
                        fontFamily: 'inherit',
                        background: 'transparent',
                        border: '1px solid var(--color-border)',
                        color: 'var(--color-success)',
                        cursor: 'pointer',
                        flexShrink: 0,
                      }}
                      onMouseEnter={e => (e.currentTarget.style.backgroundColor = 'var(--color-bg-card-hover)')}
                      onMouseLeave={e => (e.currentTarget.style.backgroundColor = 'transparent')}
                    >
                      + Issue
                    </button>
                  </div>
                </div>
              ))}
            </div>
          </div>
        )}
      </div>

      {/* Bottom event log */}
      <EventLog entries={eventLog} />

      {/* Floating action button */}
      <FloatingActionButton
        onNewIssue={handleNewIssue}
        onNewProject={handleNewProject}
        onSyncGithub={handleSyncGithub}
      />

      {/* New Issue Modal */}
      {showNewIssueModal && (
        <NewIssueModal
          projects={projects}
          defaultProjectId={newIssueProjectId}
          onSubmit={handleIssueSubmit}
          onClose={() => { setShowNewIssueModal(false); setNewIssueProjectId(null); }}
        />
      )}

      {/* Delete Confirmation Dialog */}
      {deleteConfirm && (
        <ConfirmDialog
          title="Delete project"
          message={`Delete "${deleteConfirm.name}"? This will remove all its issues and pipeline runs.`}
          confirmLabel="Delete"
          onConfirm={() => {
            handleDeleteProject(deleteConfirm.id);
            setDeleteConfirm(null);
          }}
          onCancel={() => setDeleteConfirm(null)}
        />
      )}

      {/* Project Setup Modal */}
      {showProjectSetup && (
        <div
          data-testid="project-setup-modal"
          onClick={() => setShowProjectSetup(false)}
          style={{
            position: 'fixed',
            inset: 0,
            backgroundColor: 'rgba(0,0,0,0.7)',
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            zIndex: 100,
          }}
        >
          <div onClick={e => e.stopPropagation()} style={{ width: '100%', maxWidth: '500px' }}>
            <ProjectSetup
              projects={projects}
              onSelect={handleProjectSelect}
              onCreate={handleProjectCreate}
              onClone={handleCloneProject}
            />
          </div>
        </div>
      )}
    </div>
  );
}

function App() {
  const wsUrl = `${window.location.protocol === 'https:' ? 'wss:' : 'ws:'}//${window.location.host}/ws`;
  return (
    <WebSocketProvider url={wsUrl}>
      <MissionControl />
    </WebSocketProvider>
  );
}

export default App;
