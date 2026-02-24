import { useEffect, useRef, useState, useCallback } from 'react';
import type { WsMessage } from '../types';

export type ConnectionStatus = 'connecting' | 'connected' | 'disconnected';

export function useWebSocket(url: string) {
  const [lastMessage, setLastMessage] = useState<WsMessage | null>(null);
  const [connectionStatus, setConnectionStatus] = useState<ConnectionStatus>('disconnected');
  const wsRef = useRef<WebSocket | null>(null);
  const reconnectTimeoutRef = useRef<number>(undefined);
  const reconnectAttemptRef = useRef(0);

  const connect = useCallback(() => {
    try {
      const ws = new WebSocket(url);
      wsRef.current = ws;
      setConnectionStatus('connecting');

      ws.onopen = () => {
        setConnectionStatus('connected');
        reconnectAttemptRef.current = 0;
      };

      ws.onmessage = (event) => {
        try {
          const message: WsMessage = JSON.parse(event.data);
          setLastMessage(message);
        } catch (e) {
          console.error('Failed to parse WebSocket message:', e);
        }
      };

      ws.onclose = () => {
        setConnectionStatus('disconnected');
        wsRef.current = null;
        // Exponential backoff reconnect
        const attempt = reconnectAttemptRef.current;
        const delay = Math.min(1000 * Math.pow(2, attempt), 30000);
        reconnectAttemptRef.current = attempt + 1;
        reconnectTimeoutRef.current = window.setTimeout(connect, delay);
      };

      ws.onerror = () => {
        ws.close();
      };
    } catch {
      setConnectionStatus('disconnected');
    }
  }, [url]);

  const sendMessage = useCallback((msg: any) => {
    const ws = wsRef.current;
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify(msg));
    }
  }, []);

  useEffect(() => {
    connect();
    return () => {
      if (reconnectTimeoutRef.current) {
        clearTimeout(reconnectTimeoutRef.current);
      }
      wsRef.current?.close();
    };
  }, [connect]);

  return { lastMessage, connectionStatus, sendMessage };
}
