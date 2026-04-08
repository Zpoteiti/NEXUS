import { useState, useEffect } from 'react'
import { Link } from 'react-router-dom'
import { apiRequest } from '../lib/api'
import { useAuthStore } from '../lib/store'
import { useNavigate } from 'react-router-dom'
import { ArrowLeft, Cpu, Server, Heart, Gauge } from 'lucide-react'
import { SaveButton, inputStyle, cardStyle } from '../components/shared'

type Tab = 'llm' | 'server-mcp' | 'default-soul' | 'rate-limit'

export default function AdminPage() {
  const [tab, setTab] = useState<Tab>('llm')
  const isAdmin = useAuthStore((s) => s.isAdmin)
  const navigate = useNavigate()

  useEffect(() => {
    if (!isAdmin) navigate('/chat')
  }, [isAdmin, navigate])

  const tabs: { id: Tab; label: string; icon: React.ReactNode }[] = [
    { id: 'llm', label: 'LLM Config', icon: <Cpu className="w-4 h-4" /> },
    { id: 'server-mcp', label: 'Server MCP', icon: <Server className="w-4 h-4" /> },
    { id: 'default-soul', label: 'Default Soul', icon: <Heart className="w-4 h-4" /> },
    { id: 'rate-limit', label: 'Rate Limit', icon: <Gauge className="w-4 h-4" /> },
  ]

  return (
    <div className="min-h-screen" style={{ background: '#020617' }}>
      <div className="max-w-4xl mx-auto py-8 px-4">
        <div className="flex items-center justify-between mb-6">
          <h1 className="text-2xl font-bold text-white">Admin Panel</h1>
          <Link to="/chat" className="flex items-center gap-1.5 text-sm transition-colors" style={{ color: '#64748b' }}
            onMouseEnter={e => { e.currentTarget.style.color = '#94a3b8' }}
            onMouseLeave={e => { e.currentTarget.style.color = '#64748b' }}
          >
            <ArrowLeft className="w-4 h-4" />
            Back to Chat
          </Link>
        </div>

        <div className="rounded-2xl overflow-hidden" style={cardStyle}>
          <div className="flex" style={{ borderBottom: '1px solid rgba(255,255,255,0.08)' }}>
            {tabs.map(t => (
              <button
                key={t.id}
                onClick={() => setTab(t.id)}
                className={`px-4 py-3 text-sm font-medium flex items-center gap-2 cursor-pointer transition-colors ${
                  tab === t.id ? 'border-b-2 border-indigo-500 text-indigo-400' : 'text-slate-400 hover:text-slate-200'
                }`}
              >
                {t.icon}
                {t.label}
              </button>
            ))}
          </div>
          <div className="p-6">
            {tab === 'llm' && <LlmConfigTab />}
            {tab === 'server-mcp' && <ServerMcpTab />}
            {tab === 'default-soul' && <DefaultSoulTab />}
            {tab === 'rate-limit' && <RateLimitTab />}
          </div>
        </div>
      </div>
    </div>
  )
}

function LlmConfigTab() {
  const [values, setValues] = useState<Record<string, string>>({})
  const [saved, setSaved] = useState(false)
  const [error, setError] = useState('')

  useEffect(() => {
    apiRequest('/api/llm-config').then(r => r.json()).then(data => {
      setValues({
        api_base: data.api_base?.toString() || '',
        model: data.model?.toString() || '',
        api_key: data.api_key?.toString() || '',
        context_window: data.context_window?.toString() || '',
      })
    }).catch(() => {})
  }, [])

  async function save() {
    setError('')
    const body: Record<string, unknown> = {
      api_base: values.api_base,
      model: values.model,
      api_key: values.api_key,
      context_window: parseInt(values.context_window) || 0,
    }
    const res = await apiRequest('/api/llm-config', { method: 'PUT', body: JSON.stringify(body) })
    if (res.ok) { setSaved(true); setTimeout(() => setSaved(false), 2000) }
    else { const data = await res.json().catch(() => ({})); setError(data.message || 'Failed to save') }
  }

  return (
    <div className="space-y-4">
      <div>
        <label className="block text-xs font-medium uppercase tracking-wider mb-1.5" style={{ color: '#64748b' }}>API Base URL</label>
        <input
          value={values.api_base || ''}
          onChange={e => setValues({ ...values, api_base: e.target.value })}
          className="w-full px-3 py-2.5 rounded-xl text-sm text-white placeholder-slate-500 focus:outline-none focus:ring-2 focus:ring-indigo-500/50"
          style={inputStyle}
          placeholder="e.g. https://api.openai.com/v1"
        />
      </div>
      <div>
        <label className="block text-xs font-medium uppercase tracking-wider mb-1.5" style={{ color: '#64748b' }}>Model</label>
        <input
          value={values.model || ''}
          onChange={e => setValues({ ...values, model: e.target.value })}
          className="w-full px-3 py-2.5 rounded-xl text-sm text-white placeholder-slate-500 focus:outline-none focus:ring-2 focus:ring-indigo-500/50"
          style={inputStyle}
        />
      </div>
      <div>
        <label className="block text-xs font-medium uppercase tracking-wider mb-1.5" style={{ color: '#64748b' }}>API Key</label>
        <input
          value={values.api_key || ''}
          onChange={e => setValues({ ...values, api_key: e.target.value })}
          className="w-full px-3 py-2.5 rounded-xl text-sm text-white placeholder-slate-500 focus:outline-none focus:ring-2 focus:ring-indigo-500/50"
          style={inputStyle}
        />
      </div>
      <div>
        <label className="block text-xs font-medium uppercase tracking-wider mb-1.5" style={{ color: '#64748b' }}>Context Window</label>
        <input
          value={values.context_window || ''}
          onChange={e => setValues({ ...values, context_window: e.target.value })}
          type="number"
          className="w-full px-3 py-2.5 rounded-xl text-sm text-white placeholder-slate-500 focus:outline-none focus:ring-2 focus:ring-indigo-500/50"
          style={inputStyle}
        />
      </div>
      {error && (
        <div className="text-sm p-3 rounded-xl" style={{ background: 'rgba(239, 68, 68, 0.1)', border: '1px solid rgba(239, 68, 68, 0.2)', color: '#fca5a5' }}>
          {error}
        </div>
      )}
      <SaveButton onClick={save} saved={saved} />
    </div>
  )
}

function ServerMcpTab() {
  const [config, setConfig] = useState('')
  const [saved, setSaved] = useState(false)

  useEffect(() => {
    apiRequest('/api/server-mcp').then(r => r.json()).then(d => setConfig(JSON.stringify(d, null, 2))).catch(() => {})
  }, [])

  async function save() {
    try {
      const parsed = JSON.parse(config)
      await apiRequest('/api/server-mcp', { method: 'PUT', body: JSON.stringify(parsed) })
      setSaved(true); setTimeout(() => setSaved(false), 2000)
    } catch { alert('Invalid JSON') }
  }

  return (
    <div className="space-y-3">
      <p className="text-sm" style={{ color: '#64748b' }}>Server-side MCP servers (shared across all users).</p>
      <textarea
        value={config}
        onChange={e => setConfig(e.target.value)}
        rows={12}
        className="w-full px-3 py-2 rounded-xl text-sm font-mono text-white placeholder-slate-500 focus:outline-none focus:ring-2 focus:ring-indigo-500/50"
        style={inputStyle}
      />
      <SaveButton onClick={save} saved={saved} />
    </div>
  )
}

function DefaultSoulTab() {
  const [soul, setSoul] = useState('')
  const [saved, setSaved] = useState(false)

  useEffect(() => {
    apiRequest('/api/admin/default-soul').then(r => r.json()).then(d => setSoul(d.default_soul || '')).catch(() => {})
  }, [])

  async function save() {
    await apiRequest('/api/admin/default-soul', { method: 'PUT', body: JSON.stringify({ soul }) })
    setSaved(true); setTimeout(() => setSaved(false), 2000)
  }

  return (
    <div className="space-y-3">
      <p className="text-sm" style={{ color: '#64748b' }}>Default soul/personality for new users.</p>
      <textarea
        value={soul}
        onChange={e => setSoul(e.target.value)}
        rows={10}
        className="w-full px-3 py-2 rounded-xl text-sm text-white placeholder-slate-500 focus:outline-none focus:ring-2 focus:ring-indigo-500/50"
        style={inputStyle}
      />
      <SaveButton onClick={save} saved={saved} />
    </div>
  )
}

function RateLimitTab() {
  const [limit, setLimit] = useState('')
  const [saved, setSaved] = useState(false)
  const [error, setError] = useState('')

  useEffect(() => {
    apiRequest('/api/admin/rate-limit').then(r => r.json()).then(d => {
      setLimit(d.rate_limit_per_min?.toString() || '0')
    }).catch(() => {})
  }, [])

  async function save() {
    setError('')
    const val = parseInt(limit) || 0
    const res = await apiRequest('/api/admin/rate-limit', {
      method: 'PUT',
      body: JSON.stringify({ rate_limit_per_min: val }),
    })
    if (res.ok) { setSaved(true); setTimeout(() => setSaved(false), 2000) }
    else { const data = await res.json().catch(() => ({})); setError(data.message || 'Failed to save') }
  }

  return (
    <div className="space-y-4">
      <p className="text-sm" style={{ color: '#64748b' }}>
        Maximum messages per user per minute (across all channels). Set to 0 for unlimited.
        Cron-triggered messages are always exempt from rate limiting.
      </p>
      <div>
        <label className="block text-xs font-medium uppercase tracking-wider mb-1.5" style={{ color: '#64748b' }}>Messages per minute</label>
        <input
          value={limit}
          onChange={e => setLimit(e.target.value)}
          type="number"
          min="0"
          className="w-full px-3 py-2.5 rounded-xl text-sm text-white placeholder-slate-500 focus:outline-none focus:ring-2 focus:ring-indigo-500/50"
          style={inputStyle}
          placeholder="0 (unlimited)"
        />
      </div>
      {error && (
        <div className="text-sm p-3 rounded-xl" style={{ background: 'rgba(239, 68, 68, 0.1)', border: '1px solid rgba(239, 68, 68, 0.2)', color: '#fca5a5' }}>
          {error}
        </div>
      )}
      <SaveButton onClick={save} saved={saved} />
    </div>
  )
}
