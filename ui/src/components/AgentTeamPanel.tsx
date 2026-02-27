import { useState } from 'react';
import type { AgentTeamDetail, AgentEvent } from '../types';
import { AgentCard } from './AgentCard';

interface MergeStatus {
    wave: number;
    started: boolean;
    conflicts?: boolean;
    conflictFiles?: string[];
}

interface VerificationResult {
    run_id: number;
    task_id: number;
    verification_type: string;
    passed: boolean;
    summary: string;
    screenshots: string[];
    details: any;
}

interface AgentTeamPanelProps {
    teamDetail: AgentTeamDetail;
    agentEvents: Map<number, AgentEvent[]>;
    elapsedTime: string;
    mergeStatus?: MergeStatus | null;
    verificationResults?: VerificationResult[];
}

export function AgentTeamPanel({ teamDetail, agentEvents, elapsedTime, mergeStatus, verificationResults }: AgentTeamPanelProps) {
    const [expanded, setExpanded] = useState(true);
    const [expandedScreenshot, setExpandedScreenshot] = useState<string | null>(null);
    const { team, tasks } = teamDetail;

    const waves = new Map<number, typeof tasks>();
    for (const task of tasks) {
        const wave = waves.get(task.wave) || [];
        wave.push(task);
        waves.set(task.wave, wave);
    }
    const sortedWaves = [...waves.entries()].sort(([a], [b]) => a - b);

    const completedCount = tasks.filter(t => t.status === 'completed').length;
    const failedCount = tasks.filter(t => t.status === 'failed').length;
    const totalCount = tasks.length;
    const progress = totalCount > 0 ? (completedCount / totalCount) * 100 : 0;

    const currentWave = tasks.find(t => t.status === 'running')?.wave ?? 0;

    return (
        <div className="bg-white rounded-xl border border-gray-200 shadow-sm overflow-hidden">
            <button
                onClick={() => setExpanded(!expanded)}
                className="w-full p-4 text-left hover:bg-gray-50 transition-colors"
            >
                <div className="flex items-center justify-between mb-2">
                    <div className="flex items-center gap-2">
                        <span className="text-sm font-semibold text-gray-900 truncate">
                            {team.plan_summary || 'Agent Team'}
                        </span>
                    </div>
                    <div className="flex items-center gap-3">
                        <span className="text-xs text-gray-400 tabular-nums">{elapsedTime}</span>
                        <span className="text-xs px-2 py-0.5 rounded-full bg-blue-50 text-blue-600">
                            {totalCount} agent{totalCount !== 1 ? 's' : ''}
                        </span>
                        <svg
                            className={`w-4 h-4 text-gray-400 transition-transform ${expanded ? 'rotate-180' : ''}`}
                            viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"
                        >
                            <path d="M6 9l6 6 6-6" />
                        </svg>
                    </div>
                </div>

                <div className="h-1.5 bg-gray-100 rounded-full overflow-hidden">
                    <div
                        className={`h-full rounded-full transition-all duration-500 ${
                            failedCount > 0 ? 'bg-red-500' : completedCount === totalCount ? 'bg-green-500' : 'bg-blue-500'
                        }`}
                        style={{ width: `${progress}%` }}
                    />
                </div>
                <div className="flex justify-between mt-1">
                    <span className="text-xs text-gray-400">
                        {team.strategy} | {team.isolation}
                    </span>
                    <span className="text-xs text-gray-400">
                        Wave {currentWave + 1}/{sortedWaves.length}
                    </span>
                </div>
            </button>

            {expanded && (
                <div className="border-t border-gray-100 p-4 space-y-4">
                    {sortedWaves.map(([wave, waveTasks]) => (
                        <div key={wave}>
                            <div className="text-xs font-medium text-gray-400 mb-2 uppercase tracking-wide">
                                Wave {wave + 1}
                                {waveTasks.length > 1 && ' (parallel)'}
                            </div>
                            <div className="space-y-2">
                                {waveTasks.map(task => (
                                    <AgentCard
                                        key={task.id}
                                        task={task}
                                        events={agentEvents.get(task.id) || []}
                                        defaultExpanded={task.status === 'running'}
                                    />
                                ))}
                            </div>
                        </div>
                    ))}

                    {/* Merge status indicator */}
                    {mergeStatus && (
                        <div className="pt-2 border-t border-gray-100">
                            {mergeStatus.started ? (
                                <div className="flex items-center gap-2 text-xs text-blue-600">
                                    <span className="inline-block w-2 h-2 rounded-full bg-blue-500 animate-pulse" />
                                    Merging wave {mergeStatus.wave + 1}...
                                </div>
                            ) : mergeStatus.conflicts ? (
                                <div>
                                    <div className="flex items-center gap-2 text-xs">
                                        <span className="px-2 py-0.5 rounded-full bg-red-50 text-red-600 font-medium">
                                            Merge conflict
                                        </span>
                                        <span className="text-gray-400">Wave {mergeStatus.wave + 1}</span>
                                    </div>
                                    {mergeStatus.conflictFiles && mergeStatus.conflictFiles.length > 0 && (
                                        <ul className="mt-1.5 space-y-0.5">
                                            {mergeStatus.conflictFiles.map(file => (
                                                <li key={file} className="text-xs text-red-500 font-mono pl-4">
                                                    {file}
                                                </li>
                                            ))}
                                        </ul>
                                    )}
                                </div>
                            ) : (
                                <div className="flex items-center gap-2 text-xs">
                                    <span className="px-2 py-0.5 rounded-full bg-green-50 text-green-600 font-medium">
                                        Merge complete
                                    </span>
                                    <span className="text-gray-400">Wave {mergeStatus.wave + 1}</span>
                                </div>
                            )}
                        </div>
                    )}

                    {/* Verification results */}
                    {verificationResults && verificationResults.length > 0 && (
                        <div className="pt-2 border-t border-gray-100 space-y-2">
                            <div className="text-xs font-medium text-gray-400 uppercase tracking-wide">
                                Verification
                            </div>
                            {verificationResults.map((result, i) => (
                                <div
                                    key={`${result.task_id}-${result.verification_type}-${i}`}
                                    className={`rounded-lg p-2.5 ${
                                        result.passed
                                            ? 'bg-green-50 border border-green-200'
                                            : 'bg-red-50 border border-red-200'
                                    }`}
                                >
                                    <div className="flex items-center gap-2">
                                        <svg
                                            className={`w-3.5 h-3.5 flex-shrink-0 ${result.passed ? 'text-green-500' : 'text-red-500'}`}
                                            viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5"
                                        >
                                            {result.passed
                                                ? <path d="M5 13l4 4L19 7" />
                                                : <path d="M6 18L18 6M6 6l12 12" />
                                            }
                                        </svg>
                                        <span className="text-xs font-medium text-gray-700">
                                            {result.verification_type === 'test_build' ? 'Tests & Build' : 'Visual Verification'}
                                        </span>
                                    </div>
                                    <p className="text-xs text-gray-500 mt-1 ml-6">{result.summary}</p>
                                    {result.verification_type === 'browser' && result.screenshots.length > 0 && (
                                        <div className="flex gap-1.5 flex-wrap mt-1.5 ml-6">
                                            {result.screenshots.map((src, j) => (
                                                <button
                                                    key={j}
                                                    onClick={() => setExpandedScreenshot(src)}
                                                    className="w-16 h-11 rounded border border-gray-200 overflow-hidden hover:ring-2 ring-blue-300 transition-all"
                                                >
                                                    <img
                                                        src={`/api/screenshots/${src}`}
                                                        alt={`Screenshot ${j + 1}`}
                                                        className="w-full h-full object-cover"
                                                    />
                                                </button>
                                            ))}
                                        </div>
                                    )}
                                </div>
                            ))}
                        </div>
                    )}
                </div>
            )}

            {/* Screenshot lightbox */}
            {expandedScreenshot && (
                <div
                    className="fixed inset-0 bg-black/70 z-50 flex items-center justify-center p-8"
                    onClick={() => setExpandedScreenshot(null)}
                >
                    <img
                        src={`/api/screenshots/${expandedScreenshot}`}
                        alt="Screenshot"
                        className="max-w-full max-h-full rounded-lg shadow-2xl"
                    />
                </div>
            )}
        </div>
    );
}
