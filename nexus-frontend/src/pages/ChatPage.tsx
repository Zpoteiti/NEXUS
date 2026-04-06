import { useState, useEffect, useRef } from 'react'
import { useWebSocket } from '../lib/useWebSocket'
import type { ChatMessage } from '../lib/useWebSocket'
import { apiRequest, uploadFile } from '../lib/api'
import { useAuthStore } from '../lib/store'
import { useNavigate, Link } from 'react-router-dom'
import ReactMarkdown from 'react-markdown'

interface Session {
  session_id: string
  created_at: string
}

interface Device {
  device_name: string
  last_seen_secs_ago: number
  tools_count: number
}

export default function ChatPage() {
  const { messages, progress, sessionId, connected, send, newSession, switchSession, setMessages } = useWebSocket()
  const [input, setInput] = useState('')
  const [sessions, setSessions] = useState<Session[]>([])
  const [devices, setDevices] = useState<Device[]>([])
  const [sidebarOpen, setSidebarOpen] = useState(true)
  const [pendingFiles, setPendingFiles] = useState<{ file_id: string; file_name: string }[]>([])
  const [uploading, setUploading] = useState(false)
  const fileInputRef = useRef<HTMLInputElement>(null)
  const messagesEndRef = useRef<HTMLDivElement>(null)
  const isAdmin = useAuthStore((s) => s.isAdmin)
  const logout = useAuthStore((s) => s.logout)
  const navigate = useNavigate()

  // Load sessions
  useEffect(() => {
    apiRequest('/api/sessions').then(r => r.json()).then(data => {
      setSessions(Array.isArray(data) ? data : [])
    }).catch(() => {})
  }, [sessionId])

  // Load devices periodically
  useEffect(() => {
    const load = () => {
      apiRequest('/api/devices').then(r => r.json()).then(data => {
        setDevices(Array.isArray(data) ? data : [])
      }).catch(() => {})
    }
    load()
    const interval = setInterval(load, 30000)
    return () => clearInterval(interval)
  }, [])

  // Auto-scroll
  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: 'smooth' })
  }, [messages, progress])

  // Load session history on switch
  useEffect(() => {
    if (!sessionId) return
    apiRequest(`/api/sessions/${encodeURIComponent(sessionId)}/messages`)
      .then(r => r.json())
      .then(data => {
        if (Array.isArray(data)) {
          const loaded: ChatMessage[] = data.map((m: { role: string; content: string }) => ({
            type: 'message' as const,
            content: m.content || '',
            session_id: sessionId,
            sender: m.role === 'user' ? 'user' as const : 'agent' as const,
            timestamp: Date.now(),
          })).filter((m: ChatMessage) => m.content && (m.sender === 'user' || m.sender === 'agent'))
          setMessages(loaded)
        }
      })
      .catch(() => {})
  }, [sessionId, setMessages])

  function handleSend() {
    const text = input.trim()
    if (!text) return
    const media = pendingFiles.map((f) => `${f.file_id}:${f.file_name}`)
    send(text, media.length > 0 ? media : undefined)
    setInput('')
    setPendingFiles([])
  }

  function handleKeyDown(e: React.KeyboardEvent) {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault()
      handleSend()
    }
  }

  async function handleFileSelect(e: React.ChangeEvent<HTMLInputElement>) {
    const file = e.target.files?.[0]
    if (!file) return
    setUploading(true)
    try {
      const result = await uploadFile(file)
      setPendingFiles((prev) => [...prev, { file_id: result.file_id, file_name: result.file_name }])
    } catch (err) {
      console.error('File upload failed:', err)
    } finally {
      setUploading(false)
      if (fileInputRef.current) fileInputRef.current.value = ''
    }
  }

  function removePendingFile(index: number) {
    setPendingFiles((prev) => prev.filter((_, i) => i !== index))
  }

  function handleLogout() {
    logout()
    navigate('/login')
  }

  // Determine if the current session is read-only (from external channel)
  const isReadOnly = (() => {
    if (!sessionId) return false
    return sessionId.startsWith('discord:') || sessionId.startsWith('cron:')
  })()

  const readOnlySource = (() => {
    if (!sessionId) return ''
    if (sessionId.startsWith('discord:')) return 'Discord'
    if (sessionId.startsWith('cron:')) return 'Cron'
    return ''
  })()

  function getSessionLabel(s: Session): { icon: string; label: string; readonly: boolean } {
    const id = s.session_id
    if (id.startsWith('discord:')) return { icon: '🎮', label: `Discord ${id.split(':')[1]?.slice(0, 8)}`, readonly: true }
    if (id.startsWith('cron:')) return { icon: '⏰', label: `Cron ${id.split(':')[1]?.slice(0, 8)}`, readonly: true }
    if (id.startsWith('gateway:')) return { icon: '💬', label: `Chat ${id.split(':').pop()?.slice(0, 8)}`, readonly: false }
    return { icon: '💬', label: id.slice(0, 16), readonly: false }
  }

  return (
    <div className="flex h-screen bg-gray-50">
      {/* Sidebar */}
      {sidebarOpen && (
        <div className="w-64 bg-white border-r border-gray-200 flex flex-col">
          <div className="p-4 border-b border-gray-200">
            <h2 className="text-lg font-semibold">NEXUS</h2>
          </div>

          <button
            onClick={newSession}
            className="mx-3 mt-3 px-3 py-2 bg-blue-600 text-white rounded-md hover:bg-blue-700 text-sm"
          >
            + New Chat
          </button>

          <div className="flex-1 overflow-y-auto p-3 space-y-1">
            {sessions.map((s) => {
              const { icon, label, readonly } = getSessionLabel(s)
              return (
                <button
                  key={s.session_id}
                  onClick={() => switchSession(s.session_id)}
                  className={`w-full text-left px-3 py-2 rounded-md text-sm truncate ${
                    s.session_id === sessionId
                      ? 'bg-blue-50 text-blue-700'
                      : 'hover:bg-gray-100 text-gray-700'
                  }`}
                >
                  {icon} {label}
                  {readonly && <span className="text-xs text-gray-400 ml-1">(view)</span>}
                </button>
              )
            })}
          </div>

          <div className="p-3 border-t border-gray-200 space-y-1">
            <Link to="/settings" className="block px-3 py-2 text-sm text-gray-600 hover:bg-gray-100 rounded-md">
              Settings
            </Link>
            {isAdmin && (
              <Link to="/admin" className="block px-3 py-2 text-sm text-gray-600 hover:bg-gray-100 rounded-md">
                Admin
              </Link>
            )}
            <button onClick={handleLogout} className="w-full text-left px-3 py-2 text-sm text-red-600 hover:bg-red-50 rounded-md">
              Logout
            </button>
          </div>
        </div>
      )}

      {/* Main Chat Area */}
      <div className="flex-1 flex flex-col">
        {/* Header */}
        <div className="h-14 bg-white border-b border-gray-200 flex items-center justify-between px-4">
          <div className="flex items-center gap-3">
            <button onClick={() => setSidebarOpen(!sidebarOpen)} className="text-gray-500 hover:text-gray-700">
              {sidebarOpen ? '◀' : '▶'}
            </button>
            <span className="text-sm text-gray-500">
              {connected ? '🟢 Connected' : '🔴 Disconnected'}
            </span>
          </div>

          {/* Device Status */}
          <div className="flex items-center gap-2">
            {devices.map((d) => (
              <span
                key={d.device_name}
                className="inline-flex items-center gap-1 text-xs bg-gray-100 px-2 py-1 rounded-full"
                title={`${d.tools_count} tools, last seen ${d.last_seen_secs_ago}s ago`}
              >
                <span className={d.last_seen_secs_ago < 60 ? 'text-green-500' : 'text-red-500'}>●</span>
                {d.device_name}
              </span>
            ))}
          </div>
        </div>

        {/* Messages */}
        <div className="flex-1 overflow-y-auto p-4 space-y-4">
          {messages.map((msg, i) => (
            <div
              key={i}
              className={`flex ${msg.sender === 'user' ? 'justify-end' : 'justify-start'}`}
            >
              <div
                className={`max-w-[70%] rounded-lg px-4 py-2 ${
                  msg.sender === 'user'
                    ? 'bg-blue-600 text-white'
                    : 'bg-white border border-gray-200 text-gray-800'
                }`}
              >
                {msg.sender === 'agent' ? (
                  <div className="prose prose-sm max-w-none">
                    <ReactMarkdown>{msg.content}</ReactMarkdown>
                  </div>
                ) : (
                  <p className="whitespace-pre-wrap">{msg.content}</p>
                )}
                {msg.media && msg.media.length > 0 && (
                  <div className="mt-2 space-y-2">
                    {msg.media.map((url, j) => {
                      const isImage = /\.(png|jpg|jpeg|gif|webp)(\?|$)/i.test(url)
                      return isImage ? (
                        <img key={j} src={url} alt="attachment" className="max-w-full max-h-64 rounded" />
                      ) : (
                        <a
                          key={j}
                          href={url}
                          target="_blank"
                          rel="noopener noreferrer"
                          className="inline-flex items-center gap-1 text-sm text-blue-600 hover:underline"
                        >
                          📎 Download file
                        </a>
                      )
                    })}
                  </div>
                )}
              </div>
            </div>
          ))}

          {/* Progress hint */}
          {progress && (
            <div className="flex justify-start">
              <div className="bg-gray-100 text-gray-500 text-sm rounded-lg px-4 py-2 animate-pulse">
                {progress}
              </div>
            </div>
          )}

          <div ref={messagesEndRef} />
        </div>

        {/* Input */}
        <div className="bg-white border-t border-gray-200 p-4">
          {isReadOnly ? (
            <div className="text-center text-sm text-gray-500 py-2">
              This session is read-only (from {readOnlySource})
            </div>
          ) : (
            <div>
              {pendingFiles.length > 0 && (
                <div className="flex flex-wrap gap-2 mb-2">
                  {pendingFiles.map((f, i) => (
                    <span
                      key={i}
                      className="inline-flex items-center gap-1 bg-blue-50 text-blue-700 text-xs px-2 py-1 rounded"
                    >
                      📎 {f.file_name}
                      <button
                        onClick={() => removePendingFile(i)}
                        className="ml-1 text-blue-400 hover:text-blue-600"
                      >
                        x
                      </button>
                    </span>
                  ))}
                </div>
              )}
              <div className="flex gap-2">
                <input
                  ref={fileInputRef}
                  type="file"
                  onChange={handleFileSelect}
                  className="hidden"
                />
                <button
                  onClick={() => fileInputRef.current?.click()}
                  disabled={uploading || !connected}
                  className="px-3 py-2 border border-gray-300 rounded-md hover:bg-gray-100 disabled:opacity-50 disabled:cursor-not-allowed text-gray-600"
                  title="Attach file"
                >
                  {uploading ? '...' : '📎'}
                </button>
                <textarea
                  value={input}
                  onChange={(e) => setInput(e.target.value)}
                  onKeyDown={handleKeyDown}
                  placeholder="Type a message..."
                  rows={1}
                  className="flex-1 px-3 py-2 border border-gray-300 rounded-md resize-none focus:outline-none focus:ring-2 focus:ring-blue-500"
                />
                <button
                  onClick={handleSend}
                  disabled={!input.trim() || !connected}
                  className="px-4 py-2 bg-blue-600 text-white rounded-md hover:bg-blue-700 disabled:opacity-50 disabled:cursor-not-allowed"
                >
                  Send
                </button>
              </div>
            </div>
          )}
        </div>
      </div>
    </div>
  )
}
