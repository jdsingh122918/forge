import type { ConnectionStatus } from '../hooks/useWebSocket';

interface HeaderProps {
  projectName: string | null;
  wsStatus: ConnectionStatus;
  onNewIssue: () => void;
}

export function Header({ projectName, wsStatus, onNewIssue }: HeaderProps) {
  return (
    <header className="bg-white border-b border-gray-200 px-6 py-3 flex items-center justify-between">
      <div className="flex items-center gap-4">
        <h1 className="text-lg font-bold text-gray-900 tracking-tight">Forge Factory</h1>
        {projectName && (
          <span className="text-sm text-gray-500 border-l pl-4 border-gray-300">{projectName}</span>
        )}
      </div>
      <div className="flex items-center gap-4">
        <button
          onClick={onNewIssue}
          className="px-3 py-1.5 text-sm font-medium text-white bg-blue-600 rounded-md hover:bg-blue-700 transition-colors"
        >
          + New Issue
        </button>
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
