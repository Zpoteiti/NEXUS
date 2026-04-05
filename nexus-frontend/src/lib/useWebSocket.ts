import { useState, useEffect, useRef, useCallback } from 'react'

export interface ChatMessage {
  type: 'message' | 'progress'
  content: string
  session_id: string
  sender: 'user' | 'agent'
  media?: string[]
  timestamp: number
}

interface UseWebSocketReturn {
  messages: ChatMessage[]
  progress: string | null
  sessionId: string | null
  connected: boolean
  send: (content: string) => void
  newSession: () => void
  switchSession: (sessionId: string) => void
  setMessages: React.Dispatch<React.SetStateAction<ChatMessage[]>>
}

export function useWebSocket(): UseWebSocketReturn {
  const [messages, setMessages] = useState<ChatMessage[]>([])
  const [progress, setProgress] = useState<string | null>(null)
  const [sessionId, setSessionId] = useState<string | null>(null)
  const [connected, setConnected] = useState(false)
  const ws = useRef<WebSocket | null>(null)
  const reconnectTimeout = useRef<ReturnType<typeof setTimeout>>(undefined)
  const shouldReconnect = useRef(true)

  const connect = useCallback(function connectWs() {
    if (!shouldReconnect.current) return

    const token = localStorage.getItem('jwt')
    if (!token) return

    // Clear any pending reconnect timer before creating a new connection
    if (reconnectTimeout.current) {
      clearTimeout(reconnectTimeout.current)
      reconnectTimeout.current = undefined
    }

    const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:'
    const wsUrl = `${protocol}//${window.location.host}/ws/chat?token=${token}`
    const socket = new WebSocket(wsUrl)

    socket.onopen = () => {
      setConnected(true)
    }

    socket.onmessage = (event) => {
      const data = JSON.parse(event.data)
      switch (data.type) {
        case 'message':
          setProgress(null)
          setMessages((prev) => [
            ...prev,
            {
              type: 'message',
              content: data.content,
              session_id: data.session_id,
              sender: 'agent',
              media: data.media,
              timestamp: Date.now(),
            },
          ])
          break
        case 'progress':
          setProgress(data.content)
          break
        case 'session_created':
        case 'session_switched':
          setSessionId(data.session_id)
          break
        case 'error':
          console.error('WebSocket error:', data.reason)
          break
      }
    }

    socket.onclose = () => {
      setConnected(false)
      // Only auto-reconnect if not cleaning up
      if (shouldReconnect.current) {
        reconnectTimeout.current = setTimeout(connect, 3000)
      }
    }

    socket.onerror = () => {
      socket.close()
    }

    ws.current = socket
  }, [])

  useEffect(() => {
    shouldReconnect.current = true
    connect()
    return () => {
      shouldReconnect.current = false
      if (reconnectTimeout.current) {
        clearTimeout(reconnectTimeout.current)
        reconnectTimeout.current = undefined
      }
      ws.current?.close()
    }
  }, [connect])

  const send = useCallback((content: string) => {
    if (ws.current?.readyState === WebSocket.OPEN) {
      ws.current.send(JSON.stringify({ type: 'message', content }))
      setMessages((prev) => [
        ...prev,
        {
          type: 'message',
          content,
          session_id: sessionId || '',
          sender: 'user',
          timestamp: Date.now(),
        },
      ])
    }
  }, [sessionId])

  const newSession = useCallback(() => {
    if (ws.current?.readyState === WebSocket.OPEN) {
      ws.current.send(JSON.stringify({ type: 'new_session' }))
      setMessages([])
      setProgress(null)
    }
  }, [])

  const switchSession = useCallback((sid: string) => {
    if (ws.current?.readyState === WebSocket.OPEN) {
      ws.current.send(JSON.stringify({ type: 'switch_session', session_id: sid }))
      setMessages([])
      setProgress(null)
    }
  }, [])

  return { messages, progress, sessionId, connected, send, newSession, switchSession, setMessages }
}
