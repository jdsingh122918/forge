import { useState, useRef, useEffect } from 'react';
import type { Project } from '../types';
import type { ConnectionStatus } from '../hooks/useWebSocket';

interface HeaderProps {
  project: Project | null;
  projects: Project[];
  wsStatus: ConnectionStatus;
  onNewIssue: () => void;
  onSelectProject: (project: Project) => void;
  onDisconnect: () => void;
  onSyncGithub: () => Promise<void>;
  syncing: boolean;
}

export function Header({ project, projects, wsStatus, onNewIssue, onSelectProject, onDisconnect, onSyncGithub, syncing }: HeaderProps) {
  const [open, setOpen] = useState(false);
  const menuRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const handler = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener('mousedown', handler);
    return () => document.removeEventListener('mousedown', handler);
  }, []);

  return (
    <header className="bg-white border-b border-gray-200 px-6 py-3 flex items-center justify-between">
      <div className="flex items-center gap-4">
        <h1 className="text-lg font-bold text-gray-900 tracking-tight">Forge Factory</h1>
        {project && (
          <div className="relative" ref={menuRef}>
            <button
              onClick={() => setOpen(!open)}
              className="flex items-center gap-1.5 text-sm text-gray-600 hover:text-gray-900 border-l pl-4 border-gray-300 transition-colors"
            >
              {project.name}
              <svg className="w-3.5 h-3.5 text-gray-400" viewBox="0 0 20 20" fill="currentColor">
                <path fillRule="evenodd" d="M5.23 7.21a.75.75 0 011.06.02L10 11.168l3.71-3.938a.75.75 0 111.08 1.04l-4.25 4.5a.75.75 0 01-1.08 0l-4.25-4.5a.75.75 0 01.02-1.06z" clipRule="evenodd" />
              </svg>
            </button>
            {open && (
              <div className="absolute left-0 top-full mt-1 w-64 bg-white rounded-md shadow-lg border border-gray-200 py-1 z-50">
                {projects.map((p) => (
                  <button
                    key={p.id}
                    onClick={() => { onSelectProject(p); setOpen(false); }}
                    className={`w-full text-left px-3 py-2 hover:bg-gray-50 transition-colors ${
                      p.id === project.id ? 'bg-blue-50' : ''
                    }`}
                  >
                    <div className="text-sm font-medium text-gray-900">{p.name}</div>
                    <div className="text-xs text-gray-400 font-mono truncate">{p.path}</div>
                  </button>
                ))}
                <div className="border-t border-gray-100 mt-1 pt-1">
                  <button
                    onClick={() => { onDisconnect(); setOpen(false); }}
                    className="w-full text-left px-3 py-2 text-sm text-gray-500 hover:bg-gray-50 transition-colors"
                  >
                    + Connect another project
                  </button>
                </div>
              </div>
            )}
          </div>
        )}
      </div>
      <div className="flex items-center gap-4">
        {project && (
          <button
            onClick={onSyncGithub}
            disabled={syncing}
            className="px-3 py-1.5 text-sm font-medium text-gray-700 bg-white border border-gray-300 rounded-md hover:bg-gray-50 disabled:opacity-50 transition-colors flex items-center gap-1.5"
            title="Sync issues from GitHub"
          >
            <svg className={`w-4 h-4 ${syncing ? 'animate-spin' : ''}`} viewBox="0 0 20 20" fill="currentColor">
              <path fillRule="evenodd" d="M4 2a1 1 0 011 1v2.101a7.002 7.002 0 0111.601 2.566 1 1 0 11-1.885.666A5.002 5.002 0 005.999 7H9a1 1 0 010 2H4a1 1 0 01-1-1V3a1 1 0 011-1zm.008 9.057a1 1 0 011.276.61A5.002 5.002 0 0014.001 13H11a1 1 0 110-2h5a1 1 0 011 1v5a1 1 0 11-2 0v-2.101a7.002 7.002 0 01-11.601-2.566 1 1 0 01.61-1.276z" clipRule="evenodd" />
            </svg>
            {syncing ? 'Syncing...' : 'Sync GitHub'}
          </button>
        )}
        {project && (
          <button
            onClick={onNewIssue}
            className="px-3 py-1.5 text-sm font-medium text-white bg-blue-600 rounded-md hover:bg-blue-700 transition-colors"
          >
            + New Issue
          </button>
        )}
        <div className="flex items-center gap-2">
          <span className={`inline-block w-2 h-2 rounded-full ${
            wsStatus === 'connected' ? 'bg-green-500' :
            wsStatus === 'connecting' ? 'bg-yellow-500 animate-pulse' : 'bg-red-500'
          }`} />
          <span className="text-xs text-gray-400">{wsStatus}</span>
        </div>
      </div>
    </header>
  );
}
