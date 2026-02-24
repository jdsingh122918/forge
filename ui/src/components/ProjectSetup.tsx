import { useState, useRef, useEffect, useCallback } from 'react';
import type { Project, GitHubRepo } from '../types';
import { api } from '../api/client';

type Tab = 'github' | 'local';
type GitHubState = 'idle' | 'connected';

interface ProjectSetupProps {
  projects: Project[];
  onSelect: (project: Project) => void;
  onCreate: (name: string, path: string) => void;
  onClone: (repoUrl: string) => Promise<void>;
}

export function ProjectSetup({ projects, onSelect, onCreate, onClone }: ProjectSetupProps) {
  const [tab, setTab] = useState<Tab>('github');
  const [name, setName] = useState('');
  const [path, setPath] = useState('');
  const [repoUrl, setRepoUrl] = useState('');
  const [error, setError] = useState('');
  const [cloning, setCloning] = useState(false);

  // GitHub auth state
  const [ghState, setGhState] = useState<GitHubState>('idle');
  const [repos, setRepos] = useState<GitHubRepo[]>([]);
  const [repoSearch, setRepoSearch] = useState('');
  const [selectedRepo, setSelectedRepo] = useState<GitHubRepo | null>(null);
  const [showManualInput, setShowManualInput] = useState(false);
  const [tokenInput, setTokenInput] = useState('');
  const [connectingToken, setConnectingToken] = useState(false);

  const repoRef = useRef<HTMLInputElement>(null);
  const nameRef = useRef<HTMLInputElement>(null);
  const tokenRef = useRef<HTMLInputElement>(null);

  // Check GitHub auth status on mount
  useEffect(() => {
    api.githubStatus()
      .then((s) => {
        if (s.connected) {
          setGhState('connected');
          api.githubRepos().then(setRepos).catch(console.error);
        }
      })
      .catch(console.error);
  }, []);

  useEffect(() => {
    if (tab === 'github' && showManualInput) repoRef.current?.focus();
    else if (tab === 'github' && ghState === 'idle') tokenRef.current?.focus();
    else if (tab === 'local') nameRef.current?.focus();
  }, [tab, showManualInput, ghState]);

  const handleConnectToken = useCallback(async (e: React.FormEvent) => {
    e.preventDefault();
    if (!tokenInput.trim()) return;
    setError('');
    setConnectingToken(true);
    try {
      await api.githubConnectToken(tokenInput.trim());
      setGhState('connected');
      setTokenInput('');
      const fetchedRepos = await api.githubRepos();
      setRepos(fetchedRepos);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to connect');
    } finally {
      setConnectingToken(false);
    }
  }, [tokenInput]);

  const handleDisconnect = useCallback(async () => {
    await api.githubDisconnect();
    setGhState('idle');
    setRepos([]);
    setSelectedRepo(null);
    setRepoSearch('');
  }, []);

  const handleCloneRepo = async (repo: GitHubRepo) => {
    setError('');
    setCloning(true);
    try {
      await onClone(repo.clone_url);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Clone failed');
    } finally {
      setCloning(false);
    }
  };

  const handleManualClone = async (e: React.FormEvent) => {
    e.preventDefault();
    setError('');
    if (!repoUrl.trim()) return;
    setCloning(true);
    try {
      await onClone(repoUrl.trim());
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Clone failed');
    } finally {
      setCloning(false);
    }
  };

  const handleLocal = (e: React.FormEvent) => {
    e.preventDefault();
    setError('');
    if (!name.trim() || !path.trim()) return;
    onCreate(name.trim(), path.trim());
  };

  const filteredRepos = repos.filter((r) =>
    r.full_name.toLowerCase().includes(repoSearch.toLowerCase())
  );

  const tabClass = (t: Tab) =>
    `flex-1 py-2 text-sm font-medium text-center border-b-2 transition-colors ${
      tab === t
        ? 'border-blue-600 text-blue-600'
        : 'border-transparent text-gray-500 hover:text-gray-700 hover:border-gray-300'
    }`;

  return (
    <div className="flex items-center justify-center h-full">
      <div className="w-full max-w-md space-y-6">
        <div className="bg-white rounded-lg shadow-sm border border-gray-200">
          {/* Tabs */}
          <div className="flex border-b border-gray-200">
            <button onClick={() => setTab('github')} className={tabClass('github')}>
              <span className="inline-flex items-center gap-1.5">
                <svg className="w-4 h-4" viewBox="0 0 16 16" fill="currentColor">
                  <path d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0016 8c0-4.42-3.58-8-8-8z" />
                </svg>
                GitHub
              </span>
            </button>
            <button onClick={() => setTab('local')} className={tabClass('local')}>
              <span className="inline-flex items-center gap-1.5">
                <svg className="w-4 h-4" viewBox="0 0 20 20" fill="currentColor">
                  <path fillRule="evenodd" d="M2 6a2 2 0 012-2h4l2 2h4a2 2 0 012 2v1H8a3 3 0 00-3 3v1.5a1.5 1.5 0 01-3 0V6z" clipRule="evenodd" />
                  <path d="M6 12a2 2 0 012-2h8a2 2 0 012 2v2a2 2 0 01-2 2H2h2a2 2 0 002-2v-2z" />
                </svg>
                Local path
              </span>
            </button>
          </div>

          <div className="p-6">
            {error && (
              <p className="text-sm text-red-600 bg-red-50 rounded-md px-3 py-2 mb-4">{error}</p>
            )}

            {/* GitHub tab */}
            {tab === 'github' && (
              <div className="space-y-4">
                {/* Not connected — show token input */}
                {ghState === 'idle' && !showManualInput && (
                  <>
                    <p className="text-sm text-gray-500">
                      Connect your GitHub account to browse and clone repositories.
                    </p>
                    <form onSubmit={handleConnectToken} className="space-y-3">
                      <div>
                        <label htmlFor="gh-token" className="block text-xs font-medium text-gray-600 mb-1">
                          Personal access token
                        </label>
                        <input
                          ref={tokenRef}
                          id="gh-token"
                          type="password"
                          placeholder="ghp_xxxxxxxxxxxxxxxxxxxx"
                          value={tokenInput}
                          onChange={(e) => setTokenInput(e.target.value)}
                          className="w-full px-3 py-2 border border-gray-300 rounded-md text-sm font-mono focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent"
                        />
                        <p className="text-xs text-gray-400 mt-1">
                          Create a token at{' '}
                          <a
                            href="https://github.com/settings/tokens/new?scopes=repo&description=Forge+Factory"
                            target="_blank"
                            rel="noopener noreferrer"
                            className="text-blue-500 hover:underline"
                          >
                            github.com/settings/tokens
                          </a>
                          {' '}with <code className="text-xs bg-gray-100 px-1 rounded">repo</code> scope.
                        </p>
                      </div>
                      <button
                        type="submit"
                        disabled={!tokenInput.trim() || connectingToken}
                        className="w-full px-4 py-2.5 text-sm font-medium text-white bg-gray-900 rounded-md hover:bg-gray-800 disabled:opacity-50 disabled:cursor-not-allowed transition-colors flex items-center justify-center gap-2"
                      >
                        {connectingToken ? (
                          <>
                            <svg className="w-4 h-4 animate-spin" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                              <circle cx="12" cy="12" r="10" strokeOpacity="0.25" />
                              <path d="M12 2a10 10 0 0110 10" strokeLinecap="round" />
                            </svg>
                            Connecting...
                          </>
                        ) : (
                          <>
                            <svg className="w-5 h-5" viewBox="0 0 16 16" fill="currentColor">
                              <path d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0016 8c0-4.42-3.58-8-8-8z" />
                            </svg>
                            Connect GitHub
                          </>
                        )}
                      </button>
                    </form>
                    <button
                      onClick={() => setShowManualInput(true)}
                      className="w-full text-xs text-gray-400 hover:text-gray-600 transition-colors"
                    >
                      Or clone by URL
                    </button>
                  </>
                )}

                {/* Connected — show repo picker */}
                {ghState === 'connected' && !showManualInput && (
                  <>
                    <div className="flex items-center justify-between">
                      <p className="text-sm text-green-600 font-medium flex items-center gap-1.5">
                        <svg className="w-4 h-4" viewBox="0 0 20 20" fill="currentColor">
                          <path fillRule="evenodd" d="M10 18a8 8 0 100-16 8 8 0 000 16zm3.857-9.809a.75.75 0 00-1.214-.882l-3.483 4.79-1.88-1.88a.75.75 0 10-1.06 1.061l2.5 2.5a.75.75 0 001.137-.089l4-5.5z" clipRule="evenodd" />
                        </svg>
                        GitHub connected
                      </p>
                      <button
                        onClick={handleDisconnect}
                        className="text-xs text-gray-400 hover:text-red-500 transition-colors"
                      >
                        Disconnect
                      </button>
                    </div>

                    <input
                      type="text"
                      placeholder="Search repositories..."
                      value={repoSearch}
                      onChange={(e) => { setRepoSearch(e.target.value); setSelectedRepo(null); }}
                      className="w-full px-3 py-2 border border-gray-300 rounded-md text-sm focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent"
                    />

                    <div className="max-h-56 overflow-y-auto border border-gray-200 rounded-md divide-y divide-gray-100">
                      {filteredRepos.length === 0 && (
                        <p className="text-sm text-gray-400 p-3 text-center">No repos found</p>
                      )}
                      {filteredRepos.map((repo) => (
                        <button
                          key={repo.full_name}
                          onClick={() => setSelectedRepo(repo)}
                          className={`w-full text-left px-3 py-2.5 hover:bg-blue-50 transition-colors ${
                            selectedRepo?.full_name === repo.full_name ? 'bg-blue-50 border-l-2 border-blue-500' : ''
                          }`}
                        >
                          <div className="flex items-center gap-1.5">
                            <span className="text-sm font-medium text-gray-900">{repo.full_name}</span>
                            {repo.private && (
                              <span className="text-[10px] px-1.5 py-0.5 bg-gray-100 text-gray-500 rounded-full">private</span>
                            )}
                          </div>
                          {repo.description && (
                            <p className="text-xs text-gray-400 mt-0.5 truncate">{repo.description}</p>
                          )}
                        </button>
                      ))}
                    </div>

                    <button
                      onClick={() => selectedRepo && handleCloneRepo(selectedRepo)}
                      disabled={!selectedRepo || cloning}
                      className="w-full px-4 py-2 text-sm font-medium text-white bg-blue-600 rounded-md hover:bg-blue-700 disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
                    >
                      {cloning ? 'Cloning...' : 'Clone & connect'}
                    </button>

                    <button
                      onClick={() => setShowManualInput(true)}
                      className="w-full text-xs text-gray-400 hover:text-gray-600 transition-colors"
                    >
                      Or enter URL manually
                    </button>
                  </>
                )}

                {/* Manual URL fallback (shown when user clicks "Or clone by URL") */}
                {showManualInput && (
                  <form onSubmit={handleManualClone} className="space-y-4">
                    <div className="flex items-center justify-between">
                      <p className="text-sm text-gray-500">Clone a repository by URL.</p>
                      <button
                        type="button"
                        onClick={() => setShowManualInput(false)}
                        className="text-xs text-gray-400 hover:text-gray-600 transition-colors"
                      >
                        &larr; Back
                      </button>
                    </div>
                    <input
                      ref={repoRef}
                      type="text"
                      placeholder="owner/repo or https://github.com/owner/repo"
                      value={repoUrl}
                      onChange={(e) => setRepoUrl(e.target.value)}
                      className="w-full px-3 py-2 border border-gray-300 rounded-md text-sm font-mono focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent"
                    />
                    <button
                      type="submit"
                      disabled={!repoUrl.trim() || cloning}
                      className="w-full px-4 py-2 text-sm font-medium text-white bg-blue-600 rounded-md hover:bg-blue-700 disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
                    >
                      {cloning ? 'Cloning...' : 'Clone & connect'}
                    </button>
                  </form>
                )}
              </div>
            )}

            {/* Local path tab */}
            {tab === 'local' && (
              <form onSubmit={handleLocal} className="space-y-4">
                <p className="text-sm text-gray-500">
                  Point Forge at an existing local git repository.
                </p>
                <div>
                  <label htmlFor="project-name" className="block text-sm font-medium text-gray-700 mb-1">
                    Project name
                  </label>
                  <input
                    ref={nameRef}
                    id="project-name"
                    type="text"
                    placeholder="my-app"
                    value={name}
                    onChange={(e) => setName(e.target.value)}
                    className="w-full px-3 py-2 border border-gray-300 rounded-md text-sm focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent"
                  />
                </div>
                <div>
                  <label htmlFor="project-path" className="block text-sm font-medium text-gray-700 mb-1">
                    Local path
                  </label>
                  <input
                    id="project-path"
                    type="text"
                    placeholder="/home/user/projects/my-app"
                    value={path}
                    onChange={(e) => setPath(e.target.value)}
                    className="w-full px-3 py-2 border border-gray-300 rounded-md text-sm font-mono focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent"
                  />
                  <p className="text-xs text-gray-400 mt-1">
                    Absolute path to a git repository on the host machine.
                  </p>
                </div>
                <button
                  type="submit"
                  disabled={!name.trim() || !path.trim()}
                  className="w-full px-4 py-2 text-sm font-medium text-white bg-blue-600 rounded-md hover:bg-blue-700 disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
                >
                  Connect project
                </button>
              </form>
            )}
          </div>
        </div>

        {/* Existing projects */}
        {projects.length > 0 && (
          <div className="bg-white rounded-lg shadow-sm border border-gray-200 p-6">
            <h3 className="text-sm font-medium text-gray-700 mb-3">Existing projects</h3>
            <div className="space-y-2">
              {projects.map((p) => (
                <button
                  key={p.id}
                  onClick={() => onSelect(p)}
                  className="w-full text-left px-3 py-2.5 rounded-md border border-gray-200 hover:border-blue-300 hover:bg-blue-50 transition-colors group"
                >
                  <div className="text-sm font-medium text-gray-900 group-hover:text-blue-700">{p.name}</div>
                  <div className="text-xs text-gray-400 font-mono truncate">{p.path}</div>
                </button>
              ))}
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
