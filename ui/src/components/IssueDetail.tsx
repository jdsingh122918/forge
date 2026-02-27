import { useState, useEffect, useRef, useCallback } from 'react';
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
  const [editingDescription, setEditingDescription] = useState(false);
  const [descriptionDraft, setDescriptionDraft] = useState('');
  const [newLabel, setNewLabel] = useState('');
  const [error, setError] = useState<string | null>(null);
  const titleSavedByKeyRef = useRef(false);
  const descSavedByKeyRef = useRef(false);

  const saveAndRefresh = useCallback(async (updateFn: () => Promise<unknown>) => {
    setError(null);
    try {
      await updateFn();
      const updated = await api.getIssue(issueId);
      setDetail(updated);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'An unexpected error occurred');
    }
  }, [issueId]);

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
                titleSavedByKeyRef.current = true;
                const trimmed = titleDraft.trim();
                if (trimmed && trimmed !== issue.title) {
                  await saveAndRefresh(() => api.updateIssue(issueId, { title: trimmed }));
                }
                setEditingTitle(false);
              } else if (e.key === 'Escape') {
                titleSavedByKeyRef.current = true;
                setEditingTitle(false);
              }
            }}
            onBlur={async () => {
              if (titleSavedByKeyRef.current) {
                titleSavedByKeyRef.current = false;
                return;
              }
              const trimmed = titleDraft.trim();
              if (trimmed && trimmed !== issue.title) {
                await saveAndRefresh(() => api.updateIssue(issueId, { title: trimmed }));
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

      {/* Error Banner */}
      {error && (
        <div className="mx-6 mt-4 px-3 py-2 bg-red-50 border border-red-200 rounded-md flex items-center justify-between">
          <p className="text-sm text-red-700">{error}</p>
          <button onClick={() => setError(null)} className="text-red-400 hover:text-red-600 text-lg leading-none">&times;</button>
        </div>
      )}

      {/* Body */}
      <div className="flex-1 overflow-y-auto px-6 py-4 space-y-6">
        {/* Priority & Column */}
        <div className="flex gap-3 items-center">
          <select
            className={`text-xs px-2 py-1 rounded font-medium border-none cursor-pointer ${PRIORITY_COLORS[issue.priority]}`}
            value={issue.priority}
            onChange={async (e) => {
              const newPriority = e.target.value;
              await saveAndRefresh(() => api.updateIssue(issueId, { priority: newPriority }));
            }}
          >
            <option value="low">low</option>
            <option value="medium">medium</option>
            <option value="high">high</option>
            <option value="critical">critical</option>
          </select>
          <span className="text-xs px-2 py-1 rounded font-medium bg-gray-100 text-gray-700 capitalize">
            {issue.column.replace('_', ' ')}
          </span>
        </div>

        {/* Description */}
        <div>
          <h3 className="text-sm font-medium text-gray-700 mb-1">Description</h3>
          {editingDescription ? (
            <textarea
              autoFocus
              className="w-full text-sm text-gray-600 border border-blue-400 rounded px-2 py-1 outline-none focus:ring-2 focus:ring-blue-300 min-h-[80px] resize-y"
              value={descriptionDraft}
              onChange={(e) => setDescriptionDraft(e.target.value)}
              onKeyDown={async (e) => {
                if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) {
                  e.preventDefault();
                  descSavedByKeyRef.current = true;
                  const trimmed = descriptionDraft.trim();
                  if (trimmed !== (issue.description || '')) {
                    await saveAndRefresh(() => api.updateIssue(issueId, { description: trimmed }));
                  }
                  setEditingDescription(false);
                } else if (e.key === 'Escape') {
                  descSavedByKeyRef.current = true;
                  setEditingDescription(false);
                }
              }}
              onBlur={async () => {
                if (descSavedByKeyRef.current) {
                  descSavedByKeyRef.current = false;
                  return;
                }
                const trimmed = descriptionDraft.trim();
                if (trimmed !== (issue.description || '')) {
                  await saveAndRefresh(() => api.updateIssue(issueId, { description: trimmed }));
                }
                setEditingDescription(false);
              }}
            />
          ) : (
            <p
              className="text-sm text-gray-600 whitespace-pre-wrap cursor-pointer hover:bg-gray-50 rounded px-1 py-0.5 min-h-[24px]"
              onClick={() => {
                setDescriptionDraft(issue.description || '');
                setEditingDescription(true);
              }}
            >
              {issue.description || <span className="text-gray-400 italic">Click to edit</span>}
            </p>
          )}
        </div>

        {/* Labels */}
        <div>
          <h3 className="text-sm font-medium text-gray-700 mb-1">Labels</h3>
          <div className="flex flex-wrap gap-1 items-center">
            {issue.labels.map((label) => (
              <span key={label} className="text-xs bg-gray-100 text-gray-600 px-2 py-0.5 rounded inline-flex items-center gap-1">
                {label}
                <button
                  className="text-gray-400 hover:text-red-500 leading-none"
                  onClick={async () => {
                    const updated_labels = issue.labels.filter(l => l !== label);
                    await saveAndRefresh(() => api.updateIssue(issueId, { labels: updated_labels }));
                  }}
                >&times;</button>
              </span>
            ))}
            <input
              className="text-xs border border-gray-200 rounded px-1.5 py-0.5 w-20 outline-none focus:border-blue-400"
              placeholder="Add label"
              value={newLabel}
              onChange={(e) => setNewLabel(e.target.value)}
              onKeyDown={async (e) => {
                if (e.key === 'Enter') {
                  e.preventDefault();
                  const trimmed = newLabel.trim();
                  if (trimmed && !issue.labels.includes(trimmed)) {
                    const updated_labels = [...issue.labels, trimmed];
                    await saveAndRefresh(() => api.updateIssue(issueId, { labels: updated_labels }));
                    setNewLabel('');
                  }
                }
              }}
            />
          </div>
        </div>

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
              if (window.confirm('Cancel this pipeline run? Running agents will be killed and in-progress work will be lost.')) {
                const activeRun = runs.find(r => r.status === 'queued' || r.status === 'running');
                if (activeRun) {
                  await saveAndRefresh(() => api.cancelPipelineRun(activeRun.id));
                }
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
