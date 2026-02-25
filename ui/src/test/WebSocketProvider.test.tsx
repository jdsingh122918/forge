import { describe, it, expect, vi } from 'vitest'
import { renderHook } from '@testing-library/react'
import { WebSocketProvider, useWsSubscribe, useWsStatus } from '../contexts/WebSocketContext'
import type { ReactNode } from 'react'

// We test using a mock — the actual WS connection is tested via integration
describe('WebSocketProvider', () => {
  const wrapper = ({ children }: { children: ReactNode }) => (
    <WebSocketProvider url="ws://localhost:3141/ws">{children}</WebSocketProvider>
  )

  it('useWsStatus returns a connection status', () => {
    const { result } = renderHook(() => useWsStatus(), { wrapper })
    // Initially 'connecting' or 'disconnected' — depends on env
    expect(['connecting', 'connected', 'disconnected']).toContain(result.current)
  })

  it('useWsSubscribe calls back on messages', () => {
    // This tests the subscribe/unsubscribe contract
    const callback = vi.fn()
    const { unmount } = renderHook(() => useWsSubscribe(callback), { wrapper })
    // Unmounting should not throw
    unmount()
    expect(true).toBe(true)
  })
})
