import { createContext, useContext, useEffect, useRef, useState, useCallback } from 'react'
import type { ReactNode } from 'react'
import type { WsMessage } from '../types'

export type ConnectionStatus = 'connecting' | 'connected' | 'disconnected' | 'failed'
type Subscriber = (msg: WsMessage) => void

interface WsContextValue {
  subscribe: (fn: Subscriber) => () => void
  status: ConnectionStatus
}

const WsContext = createContext<WsContextValue | null>(null)

const MAX_RECONNECT_ATTEMPTS = 20

export function WebSocketProvider({ url, children }: { url: string; children: ReactNode }) {
  const [status, setStatus] = useState<ConnectionStatus>('disconnected')
  const subscribersRef = useRef(new Set<Subscriber>())

  useEffect(() => {
    let ws: WebSocket | null = null
    let reconnectTimeout: number | undefined
    let reconnectAttempt = 0
    let cancelled = false

    function connect() {
      if (cancelled) return
      try {
        ws = new WebSocket(url)
        setStatus('connecting')

        ws.onopen = () => {
          setStatus('connected')
          reconnectAttempt = 0
        }

        ws.onmessage = (event) => {
          let message: WsMessage
          try {
            message = JSON.parse(event.data)
          } catch (err) {
            console.warn('[ws] Failed to parse WebSocket message:', err, 'Raw:', typeof event.data === 'string' ? event.data.substring(0, 200) : typeof event.data)
            return
          }
          subscribersRef.current.forEach(fn => {
            try {
              fn(message)
            } catch (err) {
              console.error('[ws] Subscriber error handling message type:', message.type, err)
            }
          })
        }

        ws.onclose = () => {
          ws = null
          if (reconnectAttempt >= MAX_RECONNECT_ATTEMPTS) {
            console.error('[ws] Max reconnection attempts reached — giving up')
            setStatus('failed')
            return
          }
          setStatus('disconnected')
          const delay = Math.min(1000 * Math.pow(2, reconnectAttempt), 30000)
          reconnectAttempt += 1
          reconnectTimeout = window.setTimeout(connect, delay)
        }

        ws.onerror = (event) => {
          console.error('[ws] WebSocket error:', event)
          ws?.close()
        }
      } catch (err) {
        console.error('[ws] Failed to create WebSocket connection:', err)
        if (reconnectAttempt >= MAX_RECONNECT_ATTEMPTS) {
          console.error('[ws] Max reconnection attempts reached — giving up')
          setStatus('failed')
          return
        }
        setStatus('disconnected')
        const delay = Math.min(1000 * Math.pow(2, reconnectAttempt), 30000)
        reconnectAttempt += 1
        reconnectTimeout = window.setTimeout(connect, delay)
      }
    }

    connect()

    return () => {
      cancelled = true
      if (reconnectTimeout) clearTimeout(reconnectTimeout)
      ws?.close()
    }
  }, [url])

  const subscribe = useCallback((fn: Subscriber) => {
    subscribersRef.current.add(fn)
    return () => { subscribersRef.current.delete(fn) }
  }, [])

  return (
    <WsContext.Provider value={{ subscribe, status }}>
      {children}
    </WsContext.Provider>
  )
}

export function useWsSubscribe(callback: Subscriber) {
  const ctx = useContext(WsContext)
  useEffect(() => {
    if (!ctx) {
      console.warn('[ws] useWsSubscribe called outside of WebSocketProvider — messages will not be received')
      return
    }
    return ctx.subscribe(callback)
  }, [ctx, callback])
}

export function useWsStatus(): ConnectionStatus {
  const ctx = useContext(WsContext)
  if (!ctx) {
    console.warn('[ws] useWsStatus called outside of WebSocketProvider')
  }
  return ctx?.status ?? 'disconnected'
}
