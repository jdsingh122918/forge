import { useState } from 'react';
import type { AgentTeamDetail, AgentEvent } from '../types';
import { AgentCard } from './AgentCard';

interface AgentTeamPanelProps {
    teamDetail: AgentTeamDetail;
    agentEvents: Map<number, AgentEvent[]>;
    elapsedTime: string;
}

export function AgentTeamPanel({ teamDetail, agentEvents, elapsedTime }: AgentTeamPanelProps) {
    const [expanded, setExpanded] = useState(true);
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
                </div>
            )}
        </div>
    );
}
