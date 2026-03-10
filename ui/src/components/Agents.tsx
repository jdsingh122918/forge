/** Agents view — displays the 4 built-in review specialist agents with their details. */
import { useState, useEffect, useCallback } from 'react';
import type { AgentInfo } from '../types';
import { api } from '../api/client';

/** Icon + color mapping per agent */
const AGENT_THEME: Record<string, { icon: string; color: string; bgGlow: string }> = {
  'security-sentinel': {
    icon: '\u{1F6E1}',
    color: '#f85149',
    bgGlow: 'rgba(248, 81, 73, 0.08)',
  },
  'performance-oracle': {
    icon: '\u{26A1}',
    color: '#d29922',
    bgGlow: 'rgba(210, 153, 34, 0.08)',
  },
  'architecture-strategist': {
    icon: '\u{1F3D7}',
    color: '#58a6ff',
    bgGlow: 'rgba(88, 166, 255, 0.08)',
  },
  'simplicity-reviewer': {
    icon: '\u{2728}',
    color: '#3fb950',
    bgGlow: 'rgba(63, 185, 80, 0.08)',
  },
};

const DEFAULT_THEME = { icon: '\u{1F916}', color: 'var(--color-text-secondary)', bgGlow: 'transparent' };

export default function Agents() {
  const [agents, setAgents] = useState<AgentInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [expandedId, setExpandedId] = useState<string | null>(null);

  const fetchAgents = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const data = await api.listAgents();
      setAgents(data);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to load agents');
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    fetchAgents();
  }, [fetchAgents]);

  if (loading) {
    return (
      <div style={{
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        height: '100%',
        color: 'var(--color-text-secondary)',
        fontSize: '13px',
        gap: '8px',
      }}>
        <span className="pulse-dot" style={{
          width: '8px',
          height: '8px',
          borderRadius: '50%',
          backgroundColor: 'var(--color-success)',
        }} />
        Loading agents...
      </div>
    );
  }

  if (error) {
    return (
      <div style={{
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        height: '100%',
        color: 'var(--color-error)',
        fontSize: '13px',
      }}>
        Failed to load agents: {error}
      </div>
    );
  }

  return (
    <div style={{ padding: '16px', overflowY: 'auto', height: '100%' }}>
      {/* Header */}
      <div style={{
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'space-between',
        marginBottom: '16px',
      }}>
        <span style={{
          fontSize: '14px',
          fontWeight: 700,
          color: 'var(--color-text-primary)',
          letterSpacing: '1px',
          textTransform: 'uppercase',
        }}>
          Review Agents
        </span>
        <span style={{
          fontSize: '12px',
          color: 'var(--color-text-secondary)',
        }}>
          {agents.length} built-in specialists
        </span>
      </div>

      {/* Agent cards */}
      <div style={{
        display: 'grid',
        gridTemplateColumns: 'repeat(auto-fill, minmax(420px, 1fr))',
        gap: '16px',
      }}>
        {agents.map(agent => {
          const theme = AGENT_THEME[agent.id] ?? DEFAULT_THEME;
          const isExpanded = expandedId === agent.id;

          return (
            <div
              key={agent.id}
              onClick={() => setExpandedId(isExpanded ? null : agent.id)}
              style={{
                backgroundColor: isExpanded ? theme.bgGlow : 'var(--color-bg-card)',
                border: `1px solid ${isExpanded ? theme.color : 'var(--color-border)'}`,
                borderLeft: `3px solid ${theme.color}`,
                cursor: 'pointer',
                transition: 'all 0.15s ease',
              }}
            >
              {/* Card header */}
              <div style={{
                display: 'flex',
                alignItems: 'center',
                padding: '16px',
                gap: '12px',
              }}>
                {/* Agent icon */}
                <span style={{ fontSize: '24px', flexShrink: 0 }}>
                  {theme.icon}
                </span>

                {/* Name + gating badge */}
                <div style={{ flex: 1, minWidth: 0 }}>
                  <div style={{
                    display: 'flex',
                    alignItems: 'center',
                    gap: '8px',
                  }}>
                    <span style={{
                      fontSize: '15px',
                      fontWeight: 700,
                      color: 'var(--color-text-primary)',
                    }}>
                      {agent.name}
                    </span>
                    <span style={{
                      fontSize: '10px',
                      padding: '2px 6px',
                      backgroundColor: agent.default_gating
                        ? 'rgba(248, 81, 73, 0.15)'
                        : 'rgba(63, 185, 80, 0.15)',
                      color: agent.default_gating
                        ? 'var(--color-error)'
                        : 'var(--color-success)',
                      fontWeight: 600,
                      textTransform: 'uppercase',
                      letterSpacing: '0.5px',
                    }}>
                      {agent.default_gating ? 'gating' : 'advisory'}
                    </span>
                  </div>
                  <div style={{
                    fontSize: '11px',
                    color: 'var(--color-text-secondary)',
                    fontFamily: 'monospace',
                    marginTop: '2px',
                  }}>
                    {agent.id}
                  </div>
                </div>

                {/* Expand chevron */}
                <span style={{
                  color: 'var(--color-text-secondary)',
                  fontSize: '14px',
                  transition: 'transform 0.15s',
                  transform: isExpanded ? 'rotate(180deg)' : 'rotate(0deg)',
                  flexShrink: 0,
                }}>
                  ▾
                </span>
              </div>

              {/* Description */}
              <div style={{
                padding: '0 16px 12px 52px',
                fontSize: '12px',
                color: 'var(--color-text-secondary)',
                lineHeight: '1.5',
              }}>
                {agent.description}
              </div>

              {/* Expanded: focus areas */}
              {isExpanded && (
                <div style={{
                  padding: '0 16px 16px 52px',
                }}>
                  <div style={{
                    fontSize: '11px',
                    fontWeight: 600,
                    color: 'var(--color-text-secondary)',
                    textTransform: 'uppercase',
                    letterSpacing: '0.5px',
                    marginBottom: '8px',
                  }}>
                    Focus Areas
                  </div>
                  <div style={{
                    display: 'flex',
                    flexWrap: 'wrap',
                    gap: '6px',
                  }}>
                    {agent.focus_areas.map((area, i) => (
                      <span
                        key={i}
                        style={{
                          fontSize: '11px',
                          padding: '3px 8px',
                          backgroundColor: 'var(--color-bg-primary)',
                          border: '1px solid var(--color-border)',
                          color: 'var(--color-text-primary)',
                        }}
                      >
                        {area}
                      </span>
                    ))}
                  </div>

                  {/* Usage hint */}
                  <div style={{
                    marginTop: '12px',
                    padding: '8px 10px',
                    backgroundColor: 'var(--color-bg-primary)',
                    border: '1px solid var(--color-border)',
                    fontSize: '11px',
                    fontFamily: 'monospace',
                    color: 'var(--color-text-secondary)',
                  }}>
                    <span style={{ color: theme.color }}>$</span> forge swarm --review {agent.id.split('-')[0]}
                  </div>
                </div>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}
