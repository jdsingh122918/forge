/** Modal dialog for creating a new issue with project selector, title, and description fields. */
import { useState } from 'react';
import type { Project } from '../types';

/** Props for the NewIssueModal component. */
export interface NewIssueModalProps {
  /** Available projects for the dropdown selector */
  projects: Project[];
  /** Handler called with (projectId, title, description) on form submission */
  onSubmit: (projectId: number, title: string, description: string) => Promise<void>;
  /** Handler called when the modal should close (backdrop click, cancel, or successful submit) */
  onClose: () => void;
  /** Optional pre-selected project ID */
  defaultProjectId?: number | null;
}

/**
 * Renders a modal overlay with a form to create a new issue.
 * Includes project selector, title input, description textarea,
 * and handles loading/error states during submission.
 */
export default function NewIssueModal({ projects, onSubmit, onClose, defaultProjectId }: NewIssueModalProps): React.JSX.Element {
  const [projectId, setProjectId] = useState<number>(defaultProjectId ?? projects[0]?.id ?? 0);
  const [title, setTitle] = useState('');
  const [description, setDescription] = useState('');
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleSubmit = async (): Promise<void> => {
    if (!title.trim() || !projectId) return;
    setSubmitting(true);
    setError(null);
    try {
      await onSubmit(projectId, title.trim(), description.trim());
      onClose();
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to create issue');
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div
      data-testid="modal-backdrop"
      onClick={onClose}
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
      <div
        onClick={e => e.stopPropagation()}
        style={{
          backgroundColor: 'var(--color-bg-card)',
          border: '1px solid var(--color-border)',
          padding: '24px',
          width: '480px',
          maxWidth: '90vw',
        }}
      >
        <h2 style={{ margin: '0 0 16px', fontSize: '16px', color: 'var(--color-text-primary)' }}>
          New Issue
        </h2>

        {error && (
          <div style={{ color: 'var(--color-error)', fontSize: '13px', marginBottom: '12px' }}>
            {error}
          </div>
        )}

        {/* Project selector */}
        <label style={{ display: 'block', marginBottom: '12px' }}>
          <span style={{ fontSize: '12px', color: 'var(--color-text-secondary)', display: 'block', marginBottom: '4px' }}>
            Project
          </span>
          <select
            value={projectId}
            onChange={e => setProjectId(Number(e.target.value))}
            style={{
              width: '100%',
              padding: '8px',
              backgroundColor: 'var(--color-bg-primary)',
              border: '1px solid var(--color-border)',
              color: 'var(--color-text-primary)',
              fontFamily: 'inherit',
              fontSize: '13px',
            }}
          >
            {projects.map(p => (
              <option key={p.id} value={p.id}>{p.name}</option>
            ))}
          </select>
        </label>

        {/* Title */}
        <label style={{ display: 'block', marginBottom: '12px' }}>
          <span style={{ fontSize: '12px', color: 'var(--color-text-secondary)', display: 'block', marginBottom: '4px' }}>
            Title
          </span>
          <input
            type="text"
            value={title}
            onChange={e => setTitle(e.target.value)}
            placeholder="What needs to be done?"
            autoFocus
            style={{
              width: '100%',
              padding: '8px',
              backgroundColor: 'var(--color-bg-primary)',
              border: '1px solid var(--color-border)',
              color: 'var(--color-text-primary)',
              fontFamily: 'inherit',
              fontSize: '13px',
              boxSizing: 'border-box',
            }}
          />
        </label>

        {/* Description */}
        <label style={{ display: 'block', marginBottom: '16px' }}>
          <span style={{ fontSize: '12px', color: 'var(--color-text-secondary)', display: 'block', marginBottom: '4px' }}>
            Description
          </span>
          <textarea
            value={description}
            onChange={e => setDescription(e.target.value)}
            placeholder="Describe the work in detail..."
            rows={6}
            style={{
              width: '100%',
              padding: '8px',
              backgroundColor: 'var(--color-bg-primary)',
              border: '1px solid var(--color-border)',
              color: 'var(--color-text-primary)',
              fontFamily: 'inherit',
              fontSize: '13px',
              resize: 'vertical',
              boxSizing: 'border-box',
            }}
          />
        </label>

        {/* Actions */}
        <div style={{ display: 'flex', justifyContent: 'flex-end', gap: '8px' }}>
          <button
            onClick={onClose}
            style={{
              padding: '8px 16px',
              background: 'transparent',
              border: '1px solid var(--color-border)',
              color: 'var(--color-text-secondary)',
              cursor: 'pointer',
              fontFamily: 'inherit',
              fontSize: '13px',
            }}
          >
            Cancel
          </button>
          <button
            onClick={handleSubmit}
            disabled={!title.trim() || !projectId || submitting}
            style={{
              padding: '8px 16px',
              backgroundColor: title.trim() && projectId ? 'var(--color-success)' : 'var(--color-border)',
              border: 'none',
              color: '#000',
              cursor: title.trim() && projectId ? 'pointer' : 'not-allowed',
              fontFamily: 'inherit',
              fontSize: '13px',
              fontWeight: 600,
            }}
          >
            {submitting ? 'Creating...' : 'Create & Run'}
          </button>
        </div>
      </div>
    </div>
  );
}
