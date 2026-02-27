/** Floating action button with expandable menu for quick actions (new issue, new project, sync). */
import { useState } from 'react';

/** Action item definition for the FAB menu. */
interface FabAction {
  /** Display label for the action */
  label: string;
  /** Click handler */
  onClick: () => void;
  /** CSS color value for the label */
  color: string;
}

/** Props for the FloatingActionButton component. */
export interface FloatingActionButtonProps {
  /** Handler called when "New Issue" is clicked */
  onNewIssue: () => void;
  /** Handler called when "New Project" is clicked */
  onNewProject: () => void;
  /** Handler called when "Sync GitHub" is clicked */
  onSyncGithub: () => void;
}

/**
 * Renders a floating "+" button that expands into a vertical menu of quick actions.
 * The "+" icon rotates 45 degrees when the menu is open.
 */
export default function FloatingActionButton({
  onNewIssue,
  onNewProject,
  onSyncGithub,
}: FloatingActionButtonProps): React.JSX.Element {
  const [open, setOpen] = useState(false);

  const actions: FabAction[] = [
    { label: 'New Issue', onClick: onNewIssue, color: 'var(--color-success)' },
    { label: 'New Project', onClick: onNewProject, color: 'var(--color-info)' },
    { label: 'Sync GitHub', onClick: onSyncGithub, color: 'var(--color-warning)' },
  ];

  return (
    <div style={{
      position: 'fixed',
      bottom: '180px',
      right: '24px',
      display: 'flex',
      flexDirection: 'column',
      alignItems: 'flex-end',
      gap: '8px',
      zIndex: 50,
    }}>
      {/* Action items */}
      {open && actions.map((action, i) => (
        <button
          key={i}
          onClick={() => {
            action.onClick();
            setOpen(false);
          }}
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: '8px',
            padding: '8px 16px',
            backgroundColor: 'var(--color-bg-card)',
            border: '1px solid var(--color-border)',
            color: action.color,
            cursor: 'pointer',
            fontSize: '13px',
            fontFamily: 'inherit',
            whiteSpace: 'nowrap',
          }}
        >
          {action.label}
        </button>
      ))}

      {/* FAB button */}
      <button
        onClick={() => setOpen(!open)}
        style={{
          width: '48px',
          height: '48px',
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          backgroundColor: 'var(--color-success)',
          border: 'none',
          color: '#000',
          cursor: 'pointer',
          fontSize: '24px',
          fontFamily: 'inherit',
          fontWeight: 700,
          transition: 'transform 0.2s',
          transform: open ? 'rotate(45deg)' : 'rotate(0deg)',
        }}
      >
        +
      </button>
    </div>
  );
}
