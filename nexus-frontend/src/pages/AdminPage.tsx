import { useState, useEffect } from 'react'
import { Link } from 'react-router-dom'
import { apiRequest } from '../lib/api'
import { useAuthStore } from '../lib/store'
import { useNavigate } from 'react-router-dom'

type Tab = 'llm' | 'embedding' | 'server-mcp' | 'default-soul'

export default function AdminPage() {
  const [tab, setTab] = useState<Tab>('llm')
  const isAdmin = useAuthStore((s) => s.isAdmin)
  const navigate = useNavigate()

  useEffect(() => {
    if (!isAdmin) navigate('/chat')
  }, [isAdmin, navigate])

  const tabs: { id: Tab; label: string }[] = [
    { id: 'llm', label: 'LLM Config' },
    { id: 'embedding', label: 'Embedding' },
    { id: 'server-mcp', label: 'Server MCP' },
    { id: 'default-soul', label: 'Default Soul' },
  ]

  return (
    <div className="min-h-screen bg-gray-50">
      <div className="max-w-4xl mx-auto py-8 px-4">
        <div className="flex items-center justify-between mb-6">
          <h1 className="text-2xl font-bold">Admin Panel</h1>
          <Link to="/chat" className="text-blue-600 hover:underline text-sm">Back to Chat</Link>
        </div>

        <div className="bg-white rounded-lg shadow">
          <div className="border-b border-gray-200 flex">
            {tabs.map(t => (
              <button key={t.id} onClick={() => setTab(t.id)}
                className={`px-4 py-3 text-sm font-medium ${tab === t.id ? 'border-b-2 border-blue-600 text-blue-600' : 'text-gray-500 hover:text-gray-700'}`}
              >{t.label}</button>
            ))}
          </div>
          <div className="p-6">
            {tab === 'llm' && <LlmConfigTab />}
            {tab === 'embedding' && <EmbeddingConfigTab />}
            {tab === 'server-mcp' && <ServerMcpTab />}
            {tab === 'default-soul' && <DefaultSoulTab />}
          </div>
        </div>
      </div>
    </div>
  )
}

function ConfigForm({ endpoint, fields }: { endpoint: string; fields: { key: string; label: string; type?: string }[] }) {
  const [values, setValues] = useState<Record<string, string>>({})
  const [saved, setSaved] = useState(false)
  const [error, setError] = useState('')

  useEffect(() => {
    apiRequest(endpoint).then(r => r.json()).then(data => {
      const v: Record<string, string> = {}
      fields.forEach(f => { v[f.key] = data[f.key]?.toString() || '' })
      setValues(v)
    }).catch(() => {})
  }, [endpoint, fields])

  async function save() {
    setError('')
    const body: Record<string, unknown> = {}
    fields.forEach(f => {
      const val = values[f.key]
      if (f.type === 'number') body[f.key] = parseInt(val) || 0
      else body[f.key] = val
    })
    const res = await apiRequest(endpoint, { method: 'PUT', body: JSON.stringify(body) })
    if (res.ok) { setSaved(true); setTimeout(() => setSaved(false), 2000) }
    else { const data = await res.json().catch(() => ({})); setError(data.message || 'Failed to save') }
  }

  return (
    <div className="space-y-3">
      {fields.map(f => (
        <div key={f.key}>
          <label className="block text-sm font-medium text-gray-700 mb-1">{f.label}</label>
          <input
            value={values[f.key] || ''}
            onChange={e => setValues({ ...values, [f.key]: e.target.value })}
            type={f.type === 'number' ? 'number' : 'text'}
            className="w-full px-3 py-2 border rounded text-sm"
          />
        </div>
      ))}
      {error && <p className="text-red-600 text-sm">{error}</p>}
      <div className="flex items-center gap-3">
        <button onClick={save} className="px-4 py-2 bg-blue-600 text-white rounded text-sm">Save</button>
        {saved && <span className="text-green-600 text-sm">Saved!</span>}
      </div>
    </div>
  )
}

function LlmConfigTab() {
  return <ConfigForm endpoint="/api/llm-config" fields={[
    { key: 'model', label: 'Model' },
    { key: 'api_base', label: 'API Base URL' },
    { key: 'api_key', label: 'API Key' },
    { key: 'context_window', label: 'Context Window', type: 'number' },
    { key: 'max_output_tokens', label: 'Max Output Tokens', type: 'number' },
  ]} />
}

function EmbeddingConfigTab() {
  return <ConfigForm endpoint="/api/embedding-config" fields={[
    { key: 'model', label: 'Model' },
    { key: 'api_base', label: 'API Base URL' },
    { key: 'api_key', label: 'API Key' },
    { key: 'max_input_length', label: 'Max Input Length', type: 'number' },
    { key: 'max_concurrency', label: 'Max Concurrency', type: 'number' },
  ]} />
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
      <p className="text-sm text-gray-500">Server-side MCP servers (shared across all users).</p>
      <textarea value={config} onChange={e => setConfig(e.target.value)} rows={12} className="w-full px-3 py-2 border rounded text-sm font-mono" />
      <div className="flex items-center gap-3">
        <button onClick={save} className="px-4 py-2 bg-blue-600 text-white rounded text-sm">Save</button>
        {saved && <span className="text-green-600 text-sm">Saved!</span>}
      </div>
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
      <p className="text-sm text-gray-500">Default soul/personality for new users.</p>
      <textarea value={soul} onChange={e => setSoul(e.target.value)} rows={10} className="w-full px-3 py-2 border rounded text-sm" />
      <div className="flex items-center gap-3">
        <button onClick={save} className="px-4 py-2 bg-blue-600 text-white rounded text-sm">Save</button>
        {saved && <span className="text-green-600 text-sm">Saved!</span>}
      </div>
    </div>
  )
}
