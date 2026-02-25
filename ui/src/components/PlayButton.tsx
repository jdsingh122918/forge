interface PlayButtonProps {
    issueId: number;
    disabled: boolean;
    loading: boolean;
    onTrigger: (issueId: number) => void;
}

export function PlayButton({ issueId, disabled, loading, onTrigger }: PlayButtonProps) {
    return (
        <button
            onClick={(e) => {
                e.stopPropagation();
                if (!disabled && !loading) {
                    onTrigger(issueId);
                }
            }}
            disabled={disabled || loading}
            className={`
                absolute top-2 right-2 w-7 h-7 rounded-full flex items-center justify-center
                transition-all duration-150 z-10
                ${disabled
                    ? 'bg-gray-100 text-gray-300 cursor-not-allowed'
                    : loading
                        ? 'bg-blue-100 text-blue-400 cursor-wait'
                        : 'bg-blue-50 text-blue-500 hover:bg-blue-500 hover:text-white hover:scale-110 cursor-pointer'
                }
            `}
            title={disabled ? 'Pipeline already running' : 'Run Pipeline'}
        >
            {loading ? (
                <svg className="w-3.5 h-3.5 animate-spin" viewBox="0 0 24 24" fill="none">
                    <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" />
                    <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z" />
                </svg>
            ) : (
                <svg className="w-3.5 h-3.5 ml-0.5" viewBox="0 0 24 24" fill="currentColor">
                    <path d="M8 5v14l11-7z" />
                </svg>
            )}
        </button>
    );
}
