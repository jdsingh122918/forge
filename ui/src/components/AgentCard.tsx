import { useState, useRef, useEffect } from 'react';
import type { AgentTask, AgentEvent } from '../types';

interface AgentCardProps {
    task: AgentTask;
    events: AgentEvent[];
    defaultExpanded?: boolean;
}

const STATUS_STYLES: Record<string, { bg: string; icon: string; pulse?: boolean }> = {
    pending: { bg: 'bg-gray-100 border-gray-200', icon: '\u23f3' },
    running: { bg: 'bg-blue-50 border-blue-200', icon: '\ud83d\udfe2', pulse: true },
    completed: { bg: 'bg-green-50 border-green-200', icon: '\u2713' },
    failed: { bg: 'bg-red-50 border-red-200', icon: '\u2717' },
};

const ROLE_LABELS: Record<string, string> = {
    planner: 'Planner',
    coder: 'Coder',
    tester: 'Tester',
    reviewer: 'Reviewer',
    browser_verifier: 'Visual Check',
    test_verifier: 'Test/Build',
};

export function AgentCard({ task, events, defaultExpanded = false }: AgentCardProps) {
    const [expanded, setExpanded] = useState(defaultExpanded);
    const outputRef = useRef<HTMLDivElement>(null);
    const style = STATUS_STYLES[task.status] || STATUS_STYLES.pending;

    const actions = events.filter(e => e.event_type === 'action');
    const thinkingEvents = events.filter(e => e.event_type === 'thinking');
    const outputEvents = events.filter(e => e.event_type === 'output');
    const lastAction = actions[actions.length - 1];

    useEffect(() => {
        if (outputRef.current && expanded) {
            outputRef.current.scrollTop = outputRef.current.scrollHeight;
        }
    }, [outputEvents.length, expanded]);

    const elapsed = task.started_at
        ? formatElapsed(new Date(task.started_at), task.completed_at ? new Date(task.completed_at) : new Date())
        : '--';

    return (
        <div className={`rounded-lg border ${style.bg} transition-all duration-200`}>
            <button
                onClick={() => setExpanded(!expanded)}
                className="w-full flex items-center gap-2 p-3 text-left"
            >
                <span className={`text-sm ${style.pulse ? 'animate-pulse' : ''}`}>
                    {style.icon}
                </span>
                <span className="text-sm font-medium flex-1 truncate">{task.name}</span>
                <span className="text-xs text-gray-400 px-1.5 py-0.5 bg-white/60 rounded">
                    {ROLE_LABELS[task.agent_role] || task.agent_role}
                </span>
                <span className="text-xs text-gray-400 tabular-nums">{elapsed}</span>
                <svg
                    className={`w-4 h-4 text-gray-400 transition-transform ${expanded ? 'rotate-180' : ''}`}
                    viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"
                >
                    <path d="M6 9l6 6 6-6" />
                </svg>
            </button>

            {!expanded && lastAction && (
                <div className="px-3 pb-2 -mt-1">
                    <span className="text-xs text-gray-500 truncate block">
                        {lastAction.content}
                    </span>
                </div>
            )}

            {expanded && (
                <div className="border-t border-gray-200/50 p-3 space-y-3">
                    {actions.length > 0 && (
                        <div>
                            <div className="text-xs font-medium text-gray-500 mb-1">
                                Actions ({actions.length})
                            </div>
                            <div className="space-y-1 max-h-32 overflow-y-auto">
                                {actions.map((event, i) => (
                                    <div key={event.id} className="flex items-start gap-1.5 text-xs">
                                        <span className={i === actions.length - 1 && task.status === 'running' ? 'text-blue-500' : 'text-green-500'}>
                                            {i === actions.length - 1 && task.status === 'running' ? '\u25cf' : '\u2713'}
                                        </span>
                                        <span className="text-gray-600 truncate">{event.content}</span>
                                    </div>
                                ))}
                            </div>
                        </div>
                    )}

                    {thinkingEvents.length > 0 && (
                        <div>
                            <div className="text-xs font-medium text-gray-500 mb-1">Thinking</div>
                            <div className="text-xs text-gray-500 bg-white/50 rounded p-2 max-h-24 overflow-y-auto font-mono leading-relaxed">
                                {thinkingEvents.map(e => e.content).join('\n').slice(-500)}
                            </div>
                        </div>
                    )}

                    {outputEvents.length > 0 && (
                        <div>
                            <div className="text-xs font-medium text-gray-500 mb-1">Output</div>
                            <div
                                ref={outputRef}
                                className="text-xs text-green-400 bg-gray-900 rounded p-2 max-h-32 overflow-y-auto font-mono leading-relaxed"
                            >
                                {outputEvents.map(e => e.content).join('\n').slice(-2000)}
                            </div>
                        </div>
                    )}

                    {task.error && (
                        <div className="text-xs text-red-600 bg-red-50 rounded p-2">
                            {task.error}
                        </div>
                    )}
                </div>
            )}
        </div>
    );
}

function formatElapsed(start: Date, end: Date): string {
    const seconds = Math.floor((end.getTime() - start.getTime()) / 1000);
    if (seconds < 60) return `${seconds}s`;
    const minutes = Math.floor(seconds / 60);
    const secs = seconds % 60;
    return `${minutes}m ${secs}s`;
}
