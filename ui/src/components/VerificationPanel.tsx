import { useState } from 'react';
import type { PipelineRun, AgentEvent } from '../types';

interface VerificationPanelProps {
    run: PipelineRun;
    verificationEvents: AgentEvent[];
}

export function VerificationPanel({ run, verificationEvents }: VerificationPanelProps) {
    const [expandedScreenshot, setExpandedScreenshot] = useState<string | null>(null);

    const testResults = verificationEvents.find(e =>
        e.metadata?.verification_type === 'test_build'
    );
    const browserResults = verificationEvents.find(e =>
        e.metadata?.verification_type === 'browser'
    );

    return (
        <div className="bg-white rounded-xl border border-gray-200 shadow-sm p-4 space-y-4">
            <div className="flex items-center justify-between">
                <span className="text-sm font-semibold text-gray-900 truncate">
                    Verification Results
                </span>
                {run.pr_url && (
                    <a
                        href={run.pr_url}
                        target="_blank"
                        rel="noopener noreferrer"
                        className="text-xs text-blue-600 hover:text-blue-800 flex items-center gap-1"
                    >
                        PR #{run.pr_url.split('/').pop()}
                        <svg className="w-3 h-3" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                            <path d="M18 13v6a2 2 0 01-2 2H5a2 2 0 01-2-2V8a2 2 0 012-2h6M15 3h6v6M10 14L21 3" />
                        </svg>
                    </a>
                )}
            </div>

            {testResults && (
                <div className={`rounded-lg p-3 ${
                    testResults.metadata?.passed ? 'bg-green-50 border border-green-200' : 'bg-red-50 border border-red-200'
                }`}>
                    <div className="flex items-center gap-2 mb-1">
                        <span>{testResults.metadata?.passed ? '\u2705' : '\u274c'}</span>
                        <span className="text-sm font-medium">Tests & Build</span>
                    </div>
                    <p className="text-xs text-gray-600">{testResults.content}</p>
                </div>
            )}

            {browserResults && (
                <div className={`rounded-lg p-3 ${
                    browserResults.metadata?.passed ? 'bg-green-50 border border-green-200' : 'bg-red-50 border border-red-200'
                }`}>
                    <div className="flex items-center gap-2 mb-1">
                        <span>{browserResults.metadata?.passed ? '\u2705' : '\u274c'}</span>
                        <span className="text-sm font-medium">Visual Verification</span>
                    </div>
                    <p className="text-xs text-gray-600 mb-2">{browserResults.content}</p>

                    {Array.isArray(browserResults.metadata?.screenshots) && (
                        <div className="flex gap-2 flex-wrap">
                            {(browserResults.metadata!.screenshots as string[]).map((src, i) => (
                                <button
                                    key={i}
                                    onClick={() => setExpandedScreenshot(src)}
                                    className="w-20 h-14 rounded border border-gray-200 overflow-hidden hover:ring-2 ring-blue-300 transition-all"
                                >
                                    <img src={`data:image/png;base64,${src}`} alt={`Screenshot ${i + 1}`} className="w-full h-full object-cover" />
                                </button>
                            ))}
                        </div>
                    )}
                </div>
            )}

            {expandedScreenshot && (
                <div
                    className="fixed inset-0 bg-black/70 z-50 flex items-center justify-center p-8"
                    onClick={() => setExpandedScreenshot(null)}
                >
                    <img
                        src={`data:image/png;base64,${expandedScreenshot}`}
                        alt="Screenshot"
                        className="max-w-full max-h-full rounded-lg shadow-2xl"
                    />
                </div>
            )}
        </div>
    );
}
