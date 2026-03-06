import { useState, useRef, useEffect, useCallback, useMemo } from 'react';
import { api } from '../api/client';
import type { CliHelpResponse } from '../types';

interface CommandAutocompleteProps {
  onCommand?: (command: string) => void;
}

interface SuggestionItem {
  kind: 'cmd' | 'opt';
  label: string;
  description: string;
}

export default function CommandAutocomplete({ onCommand }: CommandAutocompleteProps) {
  const [input, setInput] = useState('');
  const [helpData, setHelpData] = useState<CliHelpResponse | null>(null);
  const [open, setOpen] = useState(false);
  const [highlightIndex, setHighlightIndex] = useState(0);

  const containerRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  // Fetch CLI help data on mount
  useEffect(() => {
    api.cliHelp().then(setHelpData).catch(() => {});
  }, []);

  // Build flat suggestion list from help data
  const allItems = useMemo<SuggestionItem[]>(() => {
    if (!helpData) return [];
    const cmds: SuggestionItem[] = helpData.commands.map(c => ({
      kind: 'cmd',
      label: c.name,
      description: c.description,
    }));
    const opts: SuggestionItem[] = helpData.options.map(o => ({
      kind: 'opt',
      label: o.flag,
      description: o.description,
    }));
    return [...cmds, ...opts];
  }, [helpData]);

  // Filter by prefix match
  const filtered = useMemo(() => {
    if (!input) return allItems;
    const lower = input.toLowerCase();
    return allItems.filter(item => item.label.toLowerCase().startsWith(lower));
  }, [allItems, input]);

  // Ghost text: remaining chars of top match
  const ghostText = useMemo(() => {
    if (!input || filtered.length === 0) return '';
    const top = filtered[0].label;
    if (top.toLowerCase().startsWith(input.toLowerCase())) {
      return top.slice(input.length);
    }
    return '';
  }, [input, filtered]);

  // Reset highlight when filtered list changes
  useEffect(() => {
    setHighlightIndex(0);
  }, [filtered.length]);

  // Click outside to close
  useEffect(() => {
    function handleClick(e: MouseEvent) {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    }
    document.addEventListener('mousedown', handleClick);
    return () => document.removeEventListener('mousedown', handleClick);
  }, []);

  const selectItem = useCallback((item: SuggestionItem) => {
    setInput(item.label);
    setOpen(false);
    inputRef.current?.focus();
  }, []);

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'ArrowDown' && open) {
      e.preventDefault();
      setHighlightIndex(i => (i + 1) % filtered.length);
    } else if (e.key === 'ArrowUp' && open) {
      e.preventDefault();
      setHighlightIndex(i => (i - 1 + filtered.length) % filtered.length);
    } else if (e.key === 'Enter') {
      e.preventDefault();
      if (open && filtered.length > 0) {
        selectItem(filtered[highlightIndex]);
      } else if (input.trim()) {
        onCommand?.(input.trim());
        setInput('');
        setOpen(false);
      }
    } else if (e.key === 'Escape') {
      setOpen(false);
    } else if (e.key === 'Tab' && ghostText) {
      e.preventDefault();
      setInput(input + ghostText);
      setOpen(false);
    }
  };

  return (
    <div ref={containerRef} style={{ position: 'relative', flex: 1 }}>
      {/* Input + ghost text wrapper */}
      <div style={{ position: 'relative' }}>
        <input
          ref={inputRef}
          type="text"
          value={input}
          onChange={e => {
            setInput(e.target.value);
            setOpen(true);
          }}
          onFocus={() => setOpen(true)}
          onKeyDown={handleKeyDown}
          placeholder="type a command..."
          style={{
            width: '100%',
            background: 'transparent',
            border: 'none',
            outline: 'none',
            color: 'var(--color-text-primary)',
            fontFamily: 'inherit',
            fontSize: 'inherit',
          }}
        />
        {/* Ghost text overlay */}
        {ghostText && (
          <span
            aria-hidden
            style={{
              position: 'absolute',
              left: 0,
              top: 0,
              pointerEvents: 'none',
              fontFamily: 'inherit',
              fontSize: 'inherit',
              color: 'var(--color-text-secondary)',
              opacity: 0.5,
              whiteSpace: 'pre',
            }}
          >
            <span style={{ visibility: 'hidden' }}>{input}</span>
            {ghostText}
          </span>
        )}
      </div>

      {/* Dropdown */}
      {open && filtered.length > 0 && (
        <div
          style={{
            position: 'absolute',
            top: '100%',
            left: 0,
            right: 0,
            marginTop: '4px',
            backgroundColor: 'var(--color-bg-card)',
            border: '1px solid var(--color-border)',
            borderRadius: '4px',
            maxHeight: '200px',
            overflowY: 'auto',
            zIndex: 1000,
          }}
        >
          {filtered.map((item, i) => (
            <div
              key={`${item.kind}-${item.label}`}
              onMouseDown={e => {
                e.preventDefault();
                selectItem(item);
              }}
              onMouseEnter={() => setHighlightIndex(i)}
              style={{
                display: 'flex',
                alignItems: 'center',
                padding: '6px 10px',
                cursor: 'pointer',
                backgroundColor: i === highlightIndex ? 'var(--color-bg-card-hover)' : 'transparent',
                gap: '8px',
                fontSize: '13px',
              }}
            >
              <span style={{
                fontSize: '10px',
                padding: '1px 4px',
                borderRadius: '2px',
                backgroundColor: 'var(--color-border)',
                color: 'var(--color-text-secondary)',
                flexShrink: 0,
              }}>
                {item.kind}
              </span>
              <span style={{ color: 'var(--color-text-primary)', flexShrink: 0 }}>
                {item.label}
              </span>
              <span style={{
                color: 'var(--color-text-secondary)',
                opacity: 0.7,
                overflow: 'hidden',
                textOverflow: 'ellipsis',
                whiteSpace: 'nowrap',
                marginLeft: 'auto',
              }}>
                {item.description}
              </span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
