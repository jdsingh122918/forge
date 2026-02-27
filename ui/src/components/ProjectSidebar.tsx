/** Sidebar listing all projects with status indicators and filtering. */
import type { Project } from '../types';

/**
 * Props for the ProjectSidebar component.
 * @property projects - List of all projects to display
 * @property selectedProjectId - ID of the currently selected project, or null for "All Projects"
 * @property onSelectProject - Callback when a project is selected; null means "All Projects"
 * @property onDeleteProject - Callback when a project delete is requested
 * @property runsByProject - Map of project ID to running/total run counts
 */
export interface ProjectSidebarProps {
  projects: Project[];
  selectedProjectId: number | null;
  onSelectProject: (projectId: number | null) => void;
  onDeleteProject: (projectId: number) => void;
  runsByProject: Map<number, { running: number; total: number }>;
}

/** Sidebar component that displays a list of projects with status dots and run count badges. */
export default function ProjectSidebar({
  projects,
  selectedProjectId,
  onSelectProject,
  onDeleteProject,
  runsByProject,
}: ProjectSidebarProps): React.JSX.Element {
  return (
    <div style={{
      width: '200px',
      backgroundColor: 'var(--color-bg-card)',
      borderRight: '1px solid var(--color-border)',
      display: 'flex',
      flexDirection: 'column',
      overflow: 'hidden',
      flexShrink: 0,
    }}>
      {/* Header */}
      <div style={{
        padding: '12px',
        borderBottom: '1px solid var(--color-border)',
        fontSize: '11px',
        color: 'var(--color-text-secondary)',
        textTransform: 'uppercase',
        letterSpacing: '1px',
      }}>
        Projects
      </div>

      {/* Project list */}
      <div style={{ flex: 1, overflowY: 'auto', padding: '4px 0' }}>
        {/* All Projects */}
        <button
          onClick={() => onSelectProject(null)}
          style={{
            width: '100%',
            display: 'flex',
            alignItems: 'center',
            gap: '8px',
            padding: '8px 12px',
            background: selectedProjectId === null ? 'var(--color-bg-card-hover)' : 'transparent',
            border: 'none',
            borderLeft: selectedProjectId === null ? '2px solid var(--color-success)' : '2px solid transparent',
            color: 'var(--color-text-primary)',
            cursor: 'pointer',
            fontSize: '13px',
            textAlign: 'left',
            fontFamily: 'inherit',
          }}
        >
          All Projects
        </button>

        {projects.map(project => {
          const stats = runsByProject.get(project.id);
          const hasActive = stats !== undefined && stats.running > 0;
          const isSelected = selectedProjectId === project.id;

          return (
            <div
              key={project.id}
              className="project-sidebar-row"
              style={{
                display: 'flex',
                alignItems: 'center',
                position: 'relative',
              }}
            >
              <button
                onClick={() => onSelectProject(project.id)}
                style={{
                  width: '100%',
                  display: 'flex',
                  alignItems: 'center',
                  gap: '8px',
                  padding: '8px 12px',
                  paddingRight: '28px',
                  background: isSelected ? 'var(--color-bg-card-hover)' : 'transparent',
                  border: 'none',
                  borderLeft: isSelected ? '2px solid var(--color-success)' : '2px solid transparent',
                  color: 'var(--color-text-primary)',
                  cursor: 'pointer',
                  fontSize: '13px',
                  textAlign: 'left',
                  fontFamily: 'inherit',
                }}
              >
                {/* Status dot */}
                <span
                  className={hasActive ? 'pulse-dot' : undefined}
                  style={{
                    width: '6px',
                    height: '6px',
                    borderRadius: '50%',
                    backgroundColor: hasActive ? 'var(--color-success)' : 'var(--color-text-secondary)',
                    flexShrink: 0,
                  }}
                />
                {/* Name */}
                <span style={{
                  flex: 1,
                  overflow: 'hidden',
                  textOverflow: 'ellipsis',
                  whiteSpace: 'nowrap',
                }}>
                  {project.name}
                </span>
                {/* Run count badge */}
                {hasActive && (
                  <span style={{
                    fontSize: '11px',
                    padding: '1px 6px',
                    backgroundColor: 'var(--color-border)',
                    borderRadius: '8px',
                    color: 'var(--color-success)',
                  }}>
                    {stats.running}
                  </span>
                )}
              </button>
              {/* Delete button — visible on row hover */}
              <button
                className="project-delete-btn"
                onClick={(e) => {
                  e.stopPropagation();
                  if (window.confirm(`Delete project "${project.name}"? This will remove all its issues and pipeline runs.`)) {
                    onDeleteProject(project.id);
                  }
                }}
                style={{
                  position: 'absolute',
                  right: '8px',
                  top: '50%',
                  transform: 'translateY(-50%)',
                  background: 'none',
                  border: 'none',
                  color: 'var(--color-text-secondary)',
                  cursor: 'pointer',
                  fontSize: '14px',
                  lineHeight: 1,
                  padding: '2px 4px',
                  borderRadius: '3px',
                  opacity: 0,
                  transition: 'opacity 0.15s, color 0.15s',
                }}
                title="Delete project"
              >
                x
              </button>
            </div>
          );
        })}
      </div>
    </div>
  );
}
