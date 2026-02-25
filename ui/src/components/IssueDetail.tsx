import { useState, useEffect } from 'react';
import type { IssueDetail as IssueDetailType } from '../types';
import { PRIORITY_COLORS } from '../types';
import { PipelineStatus } from './PipelineStatus';
import { PhaseTimeline } from './PhaseTimeline';
import { api } from '../api/client';

interface IssueDetailProps {
  issueId: number;
  onClose: () => void;
  onTriggerPipeline: (issueId: number) => void;
  onDelete: (issueId: number) => void;
}

export function IssueDetail({ issueId, onClose, onTriggerPipeline, onDelete }: IssueDetailProps) {
  const [detail, setDetail] = useState<IssueDetailType | null>(null);
  const [loading, setLoading] = useState(true);
  const [editingTitle, setEditingTitle] = useState(false);
  const [titleDraft, setTitleDraft] = useState('');

  useEffect(() => {
    setLoading(true);
    api.getIssue(issueId)
      .then(setDetail)
      .catch(console.error)
      .finally(() => setLoading(false));
  }, [issueId]);

  if (loading) {
    return (
      <div className="fixed inset-y-0 right-0 w-96 bg-white shadow-2xl border-l border-gray-200 p-6 z-50">
        <p className="text-gray-500">Loading...</p>
      </div>
    );
  }

  if (!detail) return null;

  const { issue, runs } = detail;
  const hasActiveRun = runs.some(r => r.status === 'queued' || r.status === 'running');

  return (
    <div className="fixed inset-y-0 right-0 w-96 bg-white shadow-2xl border-l border-gray-200 z-50 flex flex-col">
      {/* Header */}
      <div className="flex items-center justify-between px-6 py-4 border-b border-gray-200">
        {editingTitle ? (
          <input
            autoFocus
            className="text-lg font-semibold text-gray-900 border border-blue-400 rounded px-1 py-0.5 flex-1 mr-2 outline-none focus:ring-2 focus:ring-blue-300"
            value={titleDraft}
            onChange={(e) => setTitleDraft(e.target.value)}
            onKeyDown={async (e) => {
              if (e.key === 'Enter') {
                e.preventDefault();
                const trimmed = titleDraft.trim();
                if (trimmed && trimmed !== issue.title) {
                  await api.updateIssue(issueId, { title: trimmed });
                  const updated = await api.getIssue(issueId);
                  setDetail(updated);
                }
                setEditingTitle(false);
              } else if (e.key === 'Escape') {
                setEditingTitle(false);
              }
            }}
            onBlur={async () => {
              const trimmed = titleDraft.trim();
              if (trimmed && trimmed !== issue.title) {
                await api.updateIssue(issueId, { title: trimmed });
                const updated = await api.getIssue(issueId);
                setDetail(updated);
              }
              setEditingTitle(false);
            }}
          />
        ) : (
          <h2
            className="text-lg font-semibold text-gray-900 truncate cursor-pointer hover:text-blue-700"
            onDoubleClick={() => {
              setTitleDraft(issue.title);
              setEditingTitle(true);
            }}
            title="Double-click to edit title"
          >
            {issue.title}
          </h2>
        )}
        <button onClick={onClose} className="text-gray-400 hover:text-gray-600 text-xl leading-none">&times;</button>
      </div>

      {/* Body */}
      <div className="flex-1 overflow-y-auto px-6 py-4 space-y-6">
        {/* Priority & Column */}
        <div className="flex gap-3">
          <span className={`text-xs px-2 py-1 rounded font-medium ${PRIORITY_COLORS[issue.priority]}`}>
            {issue.priority}
          </span>
          <span className="text-xs px-2 py-1 rounded font-medium bg-gray-100 text-gray-700 capitalize">
            {issue.column.replace('_', ' ')}
          </span>
        </div>

        {/* Description */}
        {issue.description && (
          <div>
            <h3 className="text-sm font-medium text-gray-700 mb-1">Description</h3>
            <p className="text-sm text-gray-600 whitespace-pre-wrap">{issue.description}</p>
          </div>
        )}

        {/* Labels */}
        {issue.labels.length > 0 && (
          <div>
            <h3 className="text-sm font-medium text-gray-700 mb-1">Labels</h3>
            <div className="flex flex-wrap gap-1">
              {issue.labels.map((label) => (
                <span key={label} className="text-xs bg-gray-100 text-gray-600 px-2 py-0.5 rounded">
                  {label}
                </span>
              ))}
            </div>
          </div>
        )}

        {/* Pipeline Runs */}
        <div>
          <h3 className="text-sm font-medium text-gray-700 mb-2">Pipeline Runs</h3>
          {runs.length === 0 ? (
            <p className="text-sm text-gray-400">No pipeline runs yet.</p>
          ) : (
            <div className="space-y-2">
              {runs.slice().reverse().map((run) => (
                <div key={run.id} className="bg-gray-50 rounded-md p-3 text-sm">
                  <div className="flex items-center justify-between mb-1">
                    <div className="flex items-center gap-1.5">
                      <PipelineStatus run={run} />
                      {run.has_team && (
                        <span className="text-xs px-1.5 py-0.5 rounded bg-purple-50 text-purple-600 font-medium">
                          Team
                        </span>
                      )}
                    </div>
                    <span className="text-xs text-gray-400">#{run.id}</span>
                  </div>
                  {run.branch_name && (
                    <p className="text-xs text-gray-500 mt-1 font-mono truncate" title={run.branch_name}>
                      Branch: {run.branch_name}
                    </p>
                  )}
                  {run.pr_url && (
                    <a
                      href={run.pr_url}
                      target="_blank"
                      rel="noopener noreferrer"
                      className="text-xs text-blue-600 hover:text-blue-800 mt-1 inline-block"
                      onClick={(e) => e.stopPropagation()}
                    >
                      View Pull Request
                    </a>
                  )}
                  {run.phases && run.phases.length > 0 && (
                    <div className="mt-2">
                      <PhaseTimeline phases={run.phases} />
                    </div>
                  )}
                  {run.summary && <p className="text-xs text-gray-600 mt-1">{run.summary}</p>}
                  {run.error && <p className="text-xs text-red-600 mt-1">{run.error}</p>}
                  <p className="text-xs text-gray-400 mt-1">{new Date(run.started_at).toLocaleString()}</p>
                </div>
              ))}
            </div>
          )}
        </div>

        {/* Timestamps */}
        <div className="text-xs text-gray-400 space-y-1">
          <p>Created: {new Date(issue.created_at).toLocaleString()}</p>
          <p>Updated: {new Date(issue.updated_at).toLocaleString()}</p>
        </div>
      </div>

      {/* Actions */}
      <div className="px-6 py-4 border-t border-gray-200 flex gap-2">
        <button
          onClick={() => onTriggerPipeline(issueId)}
          disabled={hasActiveRun}
          className={`flex-1 px-3 py-2 text-sm font-medium rounded-md transition-colors ${
            hasActiveRun
              ? 'bg-gray-100 text-gray-400 cursor-not-allowed'
              : 'bg-blue-600 text-white hover:bg-blue-700'
          }`}
        >
          {hasActiveRun ? 'Pipeline Running...' : 'Run Pipeline'}
        </button>
        {hasActiveRun && (
          <button
            onClick={async () => {
              const activeRun = runs.find(r => r.status === 'queued' || r.status === 'running');
              if (activeRun) {
                await api.cancelPipelineRun(activeRun.id);
                const updated = await api.getIssue(issueId);
                setDetail(updated);
              }
            }}
            className="px-3 py-2 text-sm font-medium text-orange-600 bg-orange-50 rounded-md hover:bg-orange-100 transition-colors"
          >
            Cancel
          </button>
        )}
        <button
          onClick={() => { if (confirm('Delete this issue?')) onDelete(issueId); }}
          className="px-3 py-2 text-sm font-medium text-red-600 bg-red-50 rounded-md hover:bg-red-100 transition-colors"
        >
          Delete
        </button>
      </div>
    </div>
  );
}
