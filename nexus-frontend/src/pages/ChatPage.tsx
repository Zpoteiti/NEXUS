import { useState, useEffect, useRef } from 'react'
import { useWebSocket } from '../lib/useWebSocket'
import type { ChatMessage } from '../lib/useWebSocket'
import { apiRequest, uploadFile } from '../lib/api'
import { useAuthStore } from '../lib/store'
import { useNavigate, Link } from 'react-router-dom'
import ReactMarkdown from 'react-markdown'
import { MessageSquare, Plus, Settings, Shield, LogOut, Send, Paperclip, Monitor, WifiOff, Hash, Clock, PanelLeftClose, PanelLeft, Download } from 'lucide-react'

interface Session {
  session_id: string
  created_at: string
}

interface Device {
  device_name: string
  status: 'online' | 'offline'
  last_seen_secs_ago?: number
  tools_count: number
  fs_policy?: unknown
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

  function getSessionLabel(s: Session): { icon: React.ReactNode; label: string; readonly: boolean } {
    const id = s.session_id
    if (id.startsWith('discord:')) return { icon: <Hash className="w-3.5 h-3.5" />, label: `Discord ${id.split(':')[1]?.slice(0, 8)}`, readonly: true }
    if (id.startsWith('cron:')) return { icon: <Clock className="w-3.5 h-3.5" />, label: `Cron ${id.split(':')[1]?.slice(0, 8)}`, readonly: true }
    if (id.startsWith('gateway:')) return { icon: <MessageSquare className="w-3.5 h-3.5" />, label: `Chat ${id.split(':').pop()?.slice(0, 8)}`, readonly: false }
    return { icon: <MessageSquare className="w-3.5 h-3.5" />, label: id.slice(0, 16), readonly: false }
  }

  return (
    <div className="flex h-screen" style={{ background: '#020617' }}>
      {/* Sidebar */}
      {sidebarOpen && (
        <div className="w-64 flex flex-col" style={{ background: '#0f172a', borderRight: '1px solid rgba(255,255,255,0.08)' }}>
          <div className="p-4" style={{ borderBottom: '1px solid rgba(255,255,255,0.08)' }}>
            <h2 className="text-lg font-semibold text-white tracking-tight">NEXUS</h2>
          </div>

          <button
            onClick={newSession}
            className="mx-3 mt-3 px-3 py-2.5 text-white rounded-xl text-sm font-medium flex items-center justify-center gap-2 cursor-pointer"
            style={{ background: 'linear-gradient(135deg, #6366f1, #8b5cf6)', boxShadow: '0 0 20px rgba(99, 102, 241, 0.2)' }}
          >
            <Plus className="w-4 h-4" />
            New Chat
          </button>

          <div className="flex-1 overflow-y-auto p-3 space-y-1">
            {sessions.map((s) => {
              const { icon, label, readonly } = getSessionLabel(s)
              const isActive = s.session_id === sessionId
              return (
                <button
                  key={s.session_id}
                  onClick={() => switchSession(s.session_id)}
                  className={`w-full text-left px-3 py-2 rounded-xl text-sm truncate flex items-center gap-2 cursor-pointer transition-colors ${
                    isActive ? 'text-white' : 'hover:text-slate-200'
                  }`}
                  style={{
                    background: isActive ? 'rgba(99, 102, 241, 0.15)' : 'transparent',
                    color: isActive ? '#c7d2fe' : '#94a3b8',
                    border: isActive ? '1px solid rgba(99, 102, 241, 0.25)' : '1px solid transparent',
                  }}
                >
                  {icon}
                  <span className="truncate">{label}</span>
                  {readonly && <span className="text-xs ml-auto opacity-50">(view)</span>}
                </button>
              )
            })}
          </div>

          <div className="p-3 space-y-1" style={{ borderTop: '1px solid rgba(255,255,255,0.08)' }}>
            <Link
              to="/settings"
              className="flex items-center gap-2 px-3 py-2 text-sm rounded-xl transition-colors"
              style={{ color: '#94a3b8' }}
              onMouseEnter={e => { e.currentTarget.style.background = 'rgba(255,255,255,0.05)'; e.currentTarget.style.color = '#f1f5f9' }}
              onMouseLeave={e => { e.currentTarget.style.background = 'transparent'; e.currentTarget.style.color = '#94a3b8' }}
            >
              <Settings className="w-4 h-4" />
              Settings
            </Link>
            {isAdmin && (
              <Link
                to="/admin"
                className="flex items-center gap-2 px-3 py-2 text-sm rounded-xl transition-colors"
                style={{ color: '#94a3b8' }}
                onMouseEnter={e => { e.currentTarget.style.background = 'rgba(255,255,255,0.05)'; e.currentTarget.style.color = '#f1f5f9' }}
                onMouseLeave={e => { e.currentTarget.style.background = 'transparent'; e.currentTarget.style.color = '#94a3b8' }}
              >
                <Shield className="w-4 h-4" />
                Admin
              </Link>
            )}
            <button
              onClick={handleLogout}
              className="w-full text-left flex items-center gap-2 px-3 py-2 text-sm rounded-xl cursor-pointer transition-colors"
              style={{ color: '#ef4444' }}
              onMouseEnter={e => { e.currentTarget.style.background = 'rgba(239, 68, 68, 0.1)' }}
              onMouseLeave={e => { e.currentTarget.style.background = 'transparent' }}
            >
              <LogOut className="w-4 h-4" />
              Logout
            </button>
          </div>
        </div>
      )}

      {/* Main Chat Area */}
      <div className="flex-1 flex flex-col">
        {/* Header */}
        <div className="h-14 flex items-center justify-between px-4" style={{ background: 'transparent', borderBottom: '1px solid rgba(255,255,255,0.08)' }}>
          <div className="flex items-center gap-3">
            <button
              onClick={() => setSidebarOpen(!sidebarOpen)}
              className="cursor-pointer transition-colors"
              style={{ color: '#64748b' }}
              onMouseEnter={e => { e.currentTarget.style.color = '#f1f5f9' }}
              onMouseLeave={e => { e.currentTarget.style.color = '#64748b' }}
            >
              {sidebarOpen ? <PanelLeftClose className="w-5 h-5" /> : <PanelLeft className="w-5 h-5" />}
            </button>
            <span className="flex items-center gap-2 text-sm" style={{ color: '#94a3b8' }}>
              {connected ? (
                <>
                  <span className="relative flex h-2 w-2">
                    <span className="animate-ping absolute inline-flex h-full w-full rounded-full opacity-75" style={{ background: '#22c55e' }} />
                    <span className="relative inline-flex rounded-full h-2 w-2" style={{ background: '#22c55e', boxShadow: '0 0 8px rgba(34, 197, 94, 0.5)' }} />
                  </span>
                  Connected
                </>
              ) : (
                <>
                  <WifiOff className="w-3.5 h-3.5" style={{ color: '#ef4444' }} />
                  <span style={{ color: '#ef4444' }}>Disconnected</span>
                </>
              )}
            </span>
          </div>

          {/* Device Status */}
          <div className="flex items-center gap-2">
            {devices.map((d) => {
              const dotColor = d.status === 'online' ? '#22c55e' : '#64748b'
              const dotShadow = d.status === 'online' ? '0 0 6px rgba(34, 197, 94, 0.5)' : 'none'
              const title = d.status === 'online'
                ? `${d.tools_count} tools, last seen ${d.last_seen_secs_ago ?? 0}s ago`
                : 'Offline'
              return (
                <span
                  key={d.device_name}
                  className="inline-flex items-center gap-1.5 text-xs px-2.5 py-1 rounded-full"
                  style={{
                    background: 'rgba(255,255,255,0.05)',
                    border: '1px solid rgba(255,255,255,0.08)',
                    color: '#94a3b8',
                  }}
                  title={title}
                >
                  <span
                    className="inline-block w-1.5 h-1.5 rounded-full"
                    style={{ background: dotColor, boxShadow: dotShadow }}
                  />
                  <Monitor className="w-3 h-3" />
                  <span>{d.device_name}</span>
                </span>
              )
            })}
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
                className="max-w-[70%] rounded-2xl px-4 py-2.5"
                style={msg.sender === 'user' ? {
                  background: 'linear-gradient(135deg, #6366f1, #8b5cf6)',
                  color: '#ffffff',
                  boxShadow: '0 0 20px rgba(99, 102, 241, 0.15)',
                } : {
                  background: '#0f172a',
                  border: '1px solid rgba(255,255,255,0.08)',
                  color: '#f1f5f9',
                }}
              >
                {msg.sender === 'agent' ? (
                  <div className="prose prose-sm prose-invert max-w-none">
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
                        <img key={j} src={url} alt="attachment" className="max-w-full max-h-64 rounded-xl" />
                      ) : (
                        <a
                          key={j}
                          href={url}
                          target="_blank"
                          rel="noopener noreferrer"
                          className="inline-flex items-center gap-1.5 text-sm hover:underline"
                          style={{ color: '#a5b4fc' }}
                        >
                          <Download className="w-3.5 h-3.5" />
                          Download file
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
              <div
                className="text-sm rounded-2xl px-4 py-2.5 animate-pulse"
                style={{
                  background: 'rgba(99, 102, 241, 0.1)',
                  border: '1px solid rgba(99, 102, 241, 0.2)',
                  color: '#a5b4fc',
                  boxShadow: '0 0 15px rgba(99, 102, 241, 0.1)',
                }}
              >
                {progress}
              </div>
            </div>
          )}

          <div ref={messagesEndRef} />
        </div>

        {/* Input */}
        <div className="p-4" style={{ borderTop: '1px solid rgba(255,255,255,0.08)' }}>
          {isReadOnly ? (
            <div className="text-center text-sm py-2" style={{ color: '#64748b' }}>
              This session is read-only (from {readOnlySource})
            </div>
          ) : (
            <div>
              {pendingFiles.length > 0 && (
                <div className="flex flex-wrap gap-2 mb-2">
                  {pendingFiles.map((f, i) => (
                    <span
                      key={i}
                      className="inline-flex items-center gap-1.5 text-xs px-2.5 py-1 rounded-lg"
                      style={{
                        background: 'rgba(99, 102, 241, 0.15)',
                        border: '1px solid rgba(99, 102, 241, 0.25)',
                        color: '#a5b4fc',
                      }}
                    >
                      <Paperclip className="w-3 h-3" />
                      {f.file_name}
                      <button
                        onClick={() => removePendingFile(i)}
                        className="ml-1 cursor-pointer hover:text-white transition-colors"
                        style={{ color: '#818cf8' }}
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
                  className="px-3 py-2.5 rounded-xl disabled:opacity-50 disabled:cursor-not-allowed cursor-pointer transition-colors"
                  style={{
                    background: 'rgba(255,255,255,0.05)',
                    border: '1px solid rgba(255,255,255,0.08)',
                    color: '#94a3b8',
                  }}
                  title="Attach file"
                  onMouseEnter={e => { if (!uploading && connected) { e.currentTarget.style.borderColor = 'rgba(255,255,255,0.15)'; e.currentTarget.style.color = '#f1f5f9' } }}
                  onMouseLeave={e => { e.currentTarget.style.borderColor = 'rgba(255,255,255,0.08)'; e.currentTarget.style.color = '#94a3b8' }}
                >
                  {uploading ? <span className="animate-spin">...</span> : <Paperclip className="w-4 h-4" />}
                </button>
                <textarea
                  value={input}
                  onChange={(e) => setInput(e.target.value)}
                  onKeyDown={handleKeyDown}
                  placeholder="Type a message..."
                  rows={1}
                  className="flex-1 px-3.5 py-2.5 rounded-xl resize-none text-sm text-white placeholder-slate-500 focus:outline-none focus:ring-2 focus:ring-indigo-500/50"
                  style={{
                    background: 'rgba(255,255,255,0.05)',
                    border: '1px solid rgba(255,255,255,0.08)',
                  }}
                />
                <button
                  onClick={handleSend}
                  disabled={!input.trim() || !connected}
                  className="px-4 py-2.5 text-white rounded-xl disabled:opacity-50 disabled:cursor-not-allowed cursor-pointer flex items-center gap-2 text-sm font-medium"
                  style={{
                    background: 'linear-gradient(135deg, #6366f1, #8b5cf6)',
                    boxShadow: '0 0 20px rgba(99, 102, 241, 0.2)',
                  }}
                >
                  <Send className="w-4 h-4" />
                </button>
              </div>
            </div>
          )}
        </div>
      </div>
    </div>
  )
}
