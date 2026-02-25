import { createContext, useContext, useEffect, useRef, useState, useCallback } from 'react'
import type { ReactNode } from 'react'
import type { WsMessage } from '../types'

export type ConnectionStatus = 'connecting' | 'connected' | 'disconnected'
type Subscriber = (msg: WsMessage) => void

interface WsContextValue {
  subscribe: (fn: Subscriber) => () => void
  status: ConnectionStatus
}

const WsContext = createContext<WsContextValue | null>(null)

export function WebSocketProvider({ url, children }: { url: string; children: ReactNode }) {
  const [status, setStatus] = useState<ConnectionStatus>('disconnected')
  const subscribersRef = useRef(new Set<Subscriber>())
  const wsRef = useRef<WebSocket | null>(null)
  const reconnectTimeoutRef = useRef<number>(undefined)
  const reconnectAttemptRef = useRef(0)

  const connect = useCallback(() => {
    try {
      const ws = new WebSocket(url)
      wsRef.current = ws
      setStatus('connecting')

      ws.onopen = () => {
        setStatus('connected')
        reconnectAttemptRef.current = 0
      }

      ws.onmessage = (event) => {
        let message: WsMessage
        try {
          message = JSON.parse(event.data)
        } catch {
          return
        }
        subscribersRef.current.forEach(fn => {
          try {
            fn(message)
          } catch (err) {
            console.error('[ws] Subscriber error handling message type:', (message as any)?.type, err)
          }
        })
      }

      ws.onclose = () => {
        setStatus('disconnected')
        wsRef.current = null
        const attempt = reconnectAttemptRef.current
        const delay = Math.min(1000 * Math.pow(2, attempt), 30000)
        reconnectAttemptRef.current = attempt + 1
        reconnectTimeoutRef.current = window.setTimeout(connect, delay)
      }

      ws.onerror = (event) => {
        console.error('[ws] WebSocket error:', event)
        ws.close()
      }
    } catch (err) {
      console.error('[ws] Failed to create WebSocket connection:', err)
      setStatus('disconnected')
      const delay = Math.min(1000 * Math.pow(2, reconnectAttemptRef.current), 30000)
      reconnectAttemptRef.current += 1
      reconnectTimeoutRef.current = window.setTimeout(connect, delay)
    }
  }, [url])

  useEffect(() => {
    connect()
    return () => {
      if (reconnectTimeoutRef.current) clearTimeout(reconnectTimeoutRef.current)
      wsRef.current?.close()
    }
  }, [connect])

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
    if (!ctx) return
    return ctx.subscribe(callback)
  }, [ctx, callback])
}

export function useWsStatus(): ConnectionStatus {
  const ctx = useContext(WsContext)
  return ctx?.status ?? 'disconnected'
}
