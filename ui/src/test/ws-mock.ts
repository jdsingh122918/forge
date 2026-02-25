import type { WsMessage } from '../types'

type Listener = (msg: WsMessage) => void

/**
 * Minimal in-process message bus for testing WebSocket consumers.
 * Tests push messages via `send()`, hooks receive them via `subscribe()`.
 */
export function createWsMock() {
  const listeners = new Set<Listener>()

  return {
    subscribe(fn: Listener) {
      listeners.add(fn)
      return () => { listeners.delete(fn) }
    },
    send(msg: WsMessage) {
      listeners.forEach(fn => fn(msg))
    },
    get listenerCount() { return listeners.size },
  }
}
