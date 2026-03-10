/** Analytics dashboard — summary stats, phase performance, and recent runs. */
import { useState, useEffect, useCallback } from 'react';

// ── Types ───────────────────────────────────────────────────────────

interface SummaryStats {
  total_runs: number;
  successful_runs: number;
  success_rate: number;
  avg_duration_secs: number;
  total_phases: number;
  avg_iterations_per_phase: number;
}

interface PhaseNameStats {
  phase_name: string;
  run_count: number;
  avg_iterations: number;
  avg_duration_secs: number;
  budget_utilization: number;
  success_rate: number;
}

interface RunSummary {
  run_id: string;
  issue_id: number | null;
  success: boolean;
  duration_secs: number | null;
  phases_total: number | null;
  started_at: string;
}

type TimeRange = 7 | 30 | 90;

// ── Helpers ─────────────────────────────────────────────────────────

function formatDuration(secs: number): string {
  if (secs < 60) return `${Math.round(secs)}s`;
  if (secs < 3600) return `${Math.floor(secs / 60)}m ${Math.round(secs % 60)}s`;
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  return `${h}h ${m}m`;
}

function formatPercent(rate: number): string {
  return `${(rate * 100).toFixed(1)}%`;
}

function formatDate(iso: string): string {
  const d = new Date(iso);
  return d.toLocaleDateString(undefined, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' });
}

// ── Component ───────────────────────────────────────────────────────

export default function Analytics() {
  const [timeRange, setTimeRange] = useState<TimeRange>(30);
  const [summary, setSummary] = useState<SummaryStats | null>(null);
  const [phases, setPhases] = useState<PhaseNameStats[]>([]);
  const [recentRuns, setRecentRuns] = useState<RunSummary[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const fetchData = useCallback(async (days: TimeRange) => {
    setLoading(true);
    setError(null);
    try {
      const [summaryRes, phasesRes, runsRes] = await Promise.all([
        fetch(`/api/metrics/summary?days=${days}`),
        fetch(`/api/metrics/phases?days=${days}`),
        fetch(`/api/metrics/runs/recent?limit=20`),
      ]);

      if (!summaryRes.ok || !phasesRes.ok || !runsRes.ok) {
        throw new Error('Failed to fetch metrics data');
      }

      const [summaryData, phasesData, runsData] = await Promise.all([
        summaryRes.json() as Promise<SummaryStats>,
        phasesRes.json() as Promise<PhaseNameStats[]>,
        runsRes.json() as Promise<RunSummary[]>,
      ]);

      setSummary(summaryData);
      setPhases(phasesData);
      setRecentRuns(runsData);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Unknown error');
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    fetchData(timeRange);
  }, [timeRange, fetchData]);

  // Loading state
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
        Loading analytics...
      </div>
    );
  }

  // Error state
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
        Failed to load analytics: {error}
      </div>
    );
  }

  // Empty state
  if (!summary || summary.total_runs === 0) {
    return (
      <div style={{
        display: 'flex',
        flexDirection: 'column',
        alignItems: 'center',
        justifyContent: 'center',
        height: '100%',
        gap: '8px',
      }}>
        <span style={{ color: 'var(--color-text-secondary)', fontSize: '13px' }}>
          No pipeline metrics yet
        </span>
        <span style={{ color: 'var(--color-text-secondary)', fontSize: '11px' }}>
          Run some pipelines to see analytics here
        </span>
      </div>
    );
  }

  return (
    <div style={{ padding: '16px', overflowY: 'auto', height: '100%' }}>
      {/* Header + time range selector */}
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
          Analytics
        </span>
        <div style={{ display: 'flex', gap: '4px' }}>
          {([7, 30, 90] as TimeRange[]).map(days => (
            <button
              key={days}
              onClick={() => setTimeRange(days)}
              style={{
                padding: '4px 8px',
                background: timeRange === days ? 'var(--color-border)' : 'transparent',
                border: '1px solid var(--color-border)',
                color: 'var(--color-text-primary)',
                cursor: 'pointer',
                fontSize: '12px',
              }}
            >
              {days}d
            </button>
          ))}
        </div>
      </div>

      {/* Summary cards */}
      <div style={{
        display: 'grid',
        gridTemplateColumns: 'repeat(auto-fill, minmax(180px, 1fr))',
        gap: '12px',
        marginBottom: '24px',
      }}>
        <SummaryCard label="Total Runs" value={String(summary.total_runs)} />
        <SummaryCard
          label="Success Rate"
          value={formatPercent(summary.success_rate)}
          valueColor={summary.success_rate >= 0.8 ? 'var(--color-success)' : summary.success_rate >= 0.5 ? 'var(--color-warning)' : 'var(--color-error)'}
        />
        <SummaryCard label="Avg Duration" value={formatDuration(summary.avg_duration_secs)} />
        <SummaryCard label="Total Phases" value={String(summary.total_phases)} />
        <SummaryCard label="Successful Runs" value={String(summary.successful_runs)} valueColor="var(--color-success)" />
        <SummaryCard label="Avg Iterations / Phase" value={summary.avg_iterations_per_phase.toFixed(1)} />
      </div>

      {/* Phase performance table */}
      {phases.length > 0 && (
        <div style={{ marginBottom: '24px' }}>
          <div style={{
            fontSize: '12px',
            fontWeight: 600,
            color: 'var(--color-text-secondary)',
            textTransform: 'uppercase',
            letterSpacing: '0.5px',
            marginBottom: '8px',
          }}>
            Phase Performance
          </div>
          <div style={{
            backgroundColor: 'var(--color-bg-card)',
            border: '1px solid var(--color-border)',
          }}>
            {/* Table header */}
            <div style={{
              display: 'grid',
              gridTemplateColumns: '2fr 1fr 1fr 1fr 1fr 1fr',
              padding: '8px 12px',
              borderBottom: '1px solid var(--color-border)',
              fontSize: '11px',
              fontWeight: 600,
              color: 'var(--color-text-secondary)',
              textTransform: 'uppercase',
              letterSpacing: '0.5px',
            }}>
              <span>Phase</span>
              <span style={{ textAlign: 'right' }}>Runs</span>
              <span style={{ textAlign: 'right' }}>Avg Iters</span>
              <span style={{ textAlign: 'right' }}>Avg Duration</span>
              <span style={{ textAlign: 'right' }}>Budget Use</span>
              <span style={{ textAlign: 'right' }}>Success</span>
            </div>
            {/* Table rows */}
            {phases.map(phase => (
              <div
                key={phase.phase_name}
                style={{
                  display: 'grid',
                  gridTemplateColumns: '2fr 1fr 1fr 1fr 1fr 1fr',
                  padding: '8px 12px',
                  borderBottom: '1px solid var(--color-border)',
                  fontSize: '12px',
                  color: 'var(--color-text-primary)',
                }}
              >
                <span style={{
                  overflow: 'hidden',
                  textOverflow: 'ellipsis',
                  whiteSpace: 'nowrap',
                }}>
                  {phase.phase_name}
                </span>
                <span style={{ textAlign: 'right', color: 'var(--color-text-secondary)' }}>
                  {phase.run_count}
                </span>
                <span style={{ textAlign: 'right', color: 'var(--color-text-secondary)' }}>
                  {phase.avg_iterations.toFixed(1)}
                </span>
                <span style={{ textAlign: 'right', color: 'var(--color-text-secondary)' }}>
                  {formatDuration(phase.avg_duration_secs)}
                </span>
                <span style={{ textAlign: 'right', color: 'var(--color-text-secondary)' }}>
                  {formatPercent(phase.budget_utilization)}
                </span>
                <span style={{
                  textAlign: 'right',
                  color: phase.success_rate >= 0.8 ? 'var(--color-success)' : phase.success_rate >= 0.5 ? 'var(--color-warning)' : 'var(--color-error)',
                }}>
                  {formatPercent(phase.success_rate)}
                </span>
              </div>
            ))}
          </div>
        </div>
      )}

      {/* Recent runs */}
      {recentRuns.length > 0 && (
        <div>
          <div style={{
            fontSize: '12px',
            fontWeight: 600,
            color: 'var(--color-text-secondary)',
            textTransform: 'uppercase',
            letterSpacing: '0.5px',
            marginBottom: '8px',
          }}>
            Recent Runs
          </div>
          <div style={{
            backgroundColor: 'var(--color-bg-card)',
            border: '1px solid var(--color-border)',
          }}>
            {/* Table header */}
            <div style={{
              display: 'grid',
              gridTemplateColumns: '1fr 1fr 80px 100px 80px 140px',
              padding: '8px 12px',
              borderBottom: '1px solid var(--color-border)',
              fontSize: '11px',
              fontWeight: 600,
              color: 'var(--color-text-secondary)',
              textTransform: 'uppercase',
              letterSpacing: '0.5px',
            }}>
              <span>Run ID</span>
              <span>Issue</span>
              <span style={{ textAlign: 'center' }}>Status</span>
              <span style={{ textAlign: 'right' }}>Duration</span>
              <span style={{ textAlign: 'right' }}>Phases</span>
              <span style={{ textAlign: 'right' }}>Started</span>
            </div>
            {/* Table rows */}
            {recentRuns.map(run => (
              <div
                key={run.run_id}
                style={{
                  display: 'grid',
                  gridTemplateColumns: '1fr 1fr 80px 100px 80px 140px',
                  padding: '8px 12px',
                  borderBottom: '1px solid var(--color-border)',
                  fontSize: '12px',
                  color: 'var(--color-text-primary)',
                }}
              >
                <span style={{
                  overflow: 'hidden',
                  textOverflow: 'ellipsis',
                  whiteSpace: 'nowrap',
                  fontFamily: 'monospace',
                  fontSize: '11px',
                }}>
                  {run.run_id}
                </span>
                <span style={{ color: 'var(--color-text-secondary)' }}>
                  {run.issue_id !== null ? `#${run.issue_id}` : '--'}
                </span>
                <span style={{
                  textAlign: 'center',
                  color: run.success ? 'var(--color-success)' : 'var(--color-error)',
                  fontWeight: 600,
                  fontSize: '11px',
                }}>
                  {run.success ? 'PASS' : 'FAIL'}
                </span>
                <span style={{ textAlign: 'right', color: 'var(--color-text-secondary)' }}>
                  {run.duration_secs !== null ? formatDuration(run.duration_secs) : '--'}
                </span>
                <span style={{ textAlign: 'right', color: 'var(--color-text-secondary)' }}>
                  {run.phases_total !== null ? run.phases_total : '--'}
                </span>
                <span style={{ textAlign: 'right', color: 'var(--color-text-secondary)', fontSize: '11px' }}>
                  {formatDate(run.started_at)}
                </span>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

// ── Sub-components ──────────────────────────────────────────────────

function SummaryCard({ label, value, valueColor }: {
  label: string;
  value: string;
  valueColor?: string;
}) {
  return (
    <div style={{
      backgroundColor: 'var(--color-bg-card)',
      border: '1px solid var(--color-border)',
      padding: '12px',
    }}>
      <div style={{
        fontSize: '11px',
        color: 'var(--color-text-secondary)',
        textTransform: 'uppercase',
        letterSpacing: '0.5px',
        marginBottom: '4px',
      }}>
        {label}
      </div>
      <div style={{
        fontSize: '20px',
        fontWeight: 700,
        color: valueColor ?? 'var(--color-text-primary)',
      }}>
        {value}
      </div>
    </div>
  );
}
