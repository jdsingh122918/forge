import { useState } from 'react';
import type { Project } from './types';
import { useBoard } from './hooks/useBoard';

function App() {
  const [selectedProject, _setSelectedProject] = useState<Project | null>(null);
  const { board, loading, error, wsStatus } = useBoard(selectedProject?.id ?? null);

  return (
    <div className="min-h-screen bg-gray-50">
      <header className="bg-white border-b border-gray-200 px-6 py-4">
        <div className="flex items-center justify-between">
          <h1 className="text-xl font-semibold text-gray-900">Forge Factory</h1>
          <div className="flex items-center gap-3">
            <span className={`inline-block w-2 h-2 rounded-full ${
              wsStatus === 'connected' ? 'bg-green-500' :
              wsStatus === 'connecting' ? 'bg-yellow-500' : 'bg-red-500'
            }`} />
            <span className="text-sm text-gray-500">{wsStatus}</span>
          </div>
        </div>
      </header>

      <main className="p-6">
        {loading && <p className="text-gray-500">Loading board...</p>}
        {error && <p className="text-red-500">{error}</p>}
        {!board && !loading && (
          <div className="text-center py-20">
            <h2 className="text-lg font-medium text-gray-700">No project selected</h2>
            <p className="text-gray-500 mt-2">Select or create a project to get started.</p>
          </div>
        )}
        {board && (
          <div className="grid grid-cols-5 gap-4">
            {board.columns.map((col) => (
              <div key={col.name} className="bg-gray-100 rounded-lg p-3">
                <h3 className="font-medium text-gray-700 mb-3 text-sm uppercase tracking-wide">
                  {col.name.replace('_', ' ')}
                  <span className="ml-2 text-gray-400">({col.issues.length})</span>
                </h3>
                <div className="space-y-2">
                  {col.issues.map((item) => (
                    <div key={item.issue.id} className="bg-white rounded-md p-3 shadow-sm border border-gray-200">
                      <p className="text-sm font-medium text-gray-900">{item.issue.title}</p>
                      {item.active_run && (
                        <span className="text-xs text-blue-500 mt-1 block">
                          Pipeline: {item.active_run.status}
                        </span>
                      )}
                    </div>
                  ))}
                </div>
              </div>
            ))}
          </div>
        )}
      </main>
    </div>
  );
}

export default App;
