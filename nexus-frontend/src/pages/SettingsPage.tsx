import { useState, useEffect } from 'react'
import { Link } from 'react-router-dom'
import { apiRequest } from '../lib/api'
import { ArrowLeft, User, Monitor, Zap, Heart, Brain, SlidersHorizontal, Clock, Trash2, Power, PowerOff } from 'lucide-react'

type Tab = 'profile' | 'devices' | 'skills' | 'soul' | 'memory' | 'preferences' | 'cron'

const inputStyle: React.CSSProperties = {
  background: 'rgba(255,255,255,0.05)',
  border: '1px solid rgba(255,255,255,0.08)',
}

const cardStyle: React.CSSProperties = {
  background: '#0f172a',
  border: '1px solid rgba(255,255,255,0.08)',
}

function SaveButton({ onClick, saved, label = 'Save' }: { onClick: () => void; saved: boolean; label?: string }) {
  return (
    <div className="flex items-center gap-3">
      <button
        onClick={onClick}
        className="px-4 py-2 text-white rounded-xl text-sm font-medium cursor-pointer"
        style={{ background: 'linear-gradient(135deg, #6366f1, #8b5cf6)', boxShadow: '0 0 20px rgba(99, 102, 241, 0.2)' }}
      >
        {label}
      </button>
      {saved && <span className="text-sm" style={{ color: '#22c55e' }}>Saved!</span>}
    </div>
  )
}

export default function SettingsPage() {
  const [tab, setTab] = useState<Tab>('profile')

  const tabs: { id: Tab; label: string; icon: React.ReactNode }[] = [
    { id: 'profile', label: 'Profile', icon: <User className="w-4 h-4" /> },
    { id: 'devices', label: 'Devices', icon: <Monitor className="w-4 h-4" /> },
    { id: 'skills', label: 'Skills', icon: <Zap className="w-4 h-4" /> },
    { id: 'soul', label: 'Soul', icon: <Heart className="w-4 h-4" /> },
    { id: 'memory', label: 'Memory', icon: <Brain className="w-4 h-4" /> },
    { id: 'preferences', label: 'Preferences', icon: <SlidersHorizontal className="w-4 h-4" /> },
    { id: 'cron', label: 'Cron Jobs', icon: <Clock className="w-4 h-4" /> },
  ]

  return (
    <div className="min-h-screen" style={{ background: '#020617' }}>
      <div className="max-w-4xl mx-auto py-8 px-4">
        <div className="flex items-center justify-between mb-6">
          <h1 className="text-2xl font-bold text-white">Settings</h1>
          <Link to="/chat" className="flex items-center gap-1.5 text-sm transition-colors" style={{ color: '#64748b' }}
            onMouseEnter={e => { e.currentTarget.style.color = '#94a3b8' }}
            onMouseLeave={e => { e.currentTarget.style.color = '#64748b' }}
          >
            <ArrowLeft className="w-4 h-4" />
            Back to Chat
          </Link>
        </div>

        <div className="rounded-2xl overflow-hidden" style={cardStyle}>
          <div className="flex overflow-x-auto" style={{ borderBottom: '1px solid rgba(255,255,255,0.08)' }}>
            {tabs.map(t => (
              <button
                key={t.id}
                onClick={() => setTab(t.id)}
                className={`px-4 py-3 text-sm font-medium whitespace-nowrap flex items-center gap-2 cursor-pointer transition-colors ${
                  tab === t.id ? 'border-b-2 border-indigo-500 text-indigo-400' : 'text-slate-400 hover:text-slate-200'
                }`}
              >
                {t.icon}
                {t.label}
              </button>
            ))}
          </div>

          <div className="p-6">
            {tab === 'profile' && <ProfileTab />}
            {tab === 'devices' && <DevicesTab />}
            {tab === 'skills' && <SkillsTab />}
            {tab === 'soul' && <SoulTab />}
            {tab === 'memory' && <MemoryTab />}
            {tab === 'preferences' && <PreferencesTab />}
            {tab === 'cron' && <CronTab />}
          </div>
        </div>
      </div>
    </div>
  )
}

function ProfileTab() {
  const [profile, setProfile] = useState<{ user_id: string; email: string; is_admin: boolean; created_at: string } | null>(null)

  useEffect(() => {
    apiRequest('/api/user/profile').then(r => r.json()).then(setProfile).catch(() => {})
  }, [])

  if (!profile) return <p style={{ color: '#64748b' }}>Loading...</p>

  return (
    <div className="space-y-4">
      <div>
        <label className="block text-xs font-medium uppercase tracking-wider mb-1" style={{ color: '#64748b' }}>Email</label>
        <p className="text-lg text-white">{profile.email}</p>
      </div>
      <div>
        <label className="block text-xs font-medium uppercase tracking-wider mb-1" style={{ color: '#64748b' }}>User ID</label>
        <p className="text-sm font-mono" style={{ color: '#94a3b8' }}>{profile.user_id}</p>
      </div>
      <div>
        <label className="block text-xs font-medium uppercase tracking-wider mb-1" style={{ color: '#64748b' }}>Role</label>
        <p className="text-white">{profile.is_admin ? 'Admin' : 'User'}</p>
      </div>
      <div>
        <label className="block text-xs font-medium uppercase tracking-wider mb-1" style={{ color: '#64748b' }}>Created</label>
        <p className="text-white">{new Date(profile.created_at).toLocaleDateString()}</p>
      </div>
    </div>
  )
}

function DevicesTab() {
  const [devices, setDevices] = useState<Array<{ device_name: string; device_key: string; last_seen_secs_ago: number; tools_count: number }>>([])
  const [tokens, setTokens] = useState<Array<{ token: string; device_name: string; created_at: string }>>([])
  const [newName, setNewName] = useState('')

  useEffect(() => {
    apiRequest('/api/devices').then(r => r.json()).then(d => setDevices(Array.isArray(d) ? d : [])).catch(() => {})
    apiRequest('/api/device-tokens').then(r => r.json()).then(t => setTokens(Array.isArray(t) ? t : [])).catch(() => {})
  }, [])

  async function createToken() {
    if (!newName.trim()) return
    await apiRequest('/api/device-tokens', { method: 'POST', body: JSON.stringify({ device_name: newName }) })
    setNewName('')
    apiRequest('/api/device-tokens').then(r => r.json()).then(setTokens).catch(() => {})
  }

  return (
    <div className="space-y-6">
      <div>
        <h3 className="font-medium text-white mb-3">Online Devices</h3>
        {devices.length === 0 ? <p className="text-sm" style={{ color: '#64748b' }}>No devices connected</p> : (
          <table className="w-full text-sm">
            <thead>
              <tr style={{ color: '#64748b' }} className="text-left">
                <th className="pb-2 font-medium">Name</th>
                <th className="pb-2 font-medium">Tools</th>
                <th className="pb-2 font-medium">Last Seen</th>
              </tr>
            </thead>
            <tbody>{devices.map(d => (
              <tr key={d.device_name} style={{ borderTop: '1px solid rgba(255,255,255,0.08)' }}>
                <td className="py-2 text-white">{d.device_name}</td>
                <td className="py-2" style={{ color: '#94a3b8' }}>{d.tools_count}</td>
                <td className="py-2" style={{ color: d.last_seen_secs_ago < 60 ? '#22c55e' : '#ef4444' }}>
                  {d.last_seen_secs_ago < 60 ? 'Online' : `${Math.round(d.last_seen_secs_ago / 60)}m ago`}
                </td>
              </tr>
            ))}</tbody>
          </table>
        )}
      </div>

      <div>
        <h3 className="font-medium text-white mb-3">Device Tokens</h3>
        <div className="flex gap-2 mb-3">
          <input
            value={newName}
            onChange={e => setNewName(e.target.value)}
            placeholder="Device name"
            className="flex-1 px-3 py-2 rounded-xl text-sm text-white placeholder-slate-500 focus:outline-none focus:ring-2 focus:ring-indigo-500/50"
            style={inputStyle}
          />
          <button
            onClick={createToken}
            className="px-4 py-2 text-white rounded-xl text-sm font-medium cursor-pointer"
            style={{ background: 'linear-gradient(135deg, #6366f1, #8b5cf6)' }}
          >
            Create
          </button>
        </div>
        {tokens.map(t => (
          <div key={t.token} className="flex justify-between items-center py-2 text-sm" style={{ borderTop: '1px solid rgba(255,255,255,0.08)' }}>
            <span className="text-white">
              {t.device_name}{' '}
              <code className="text-xs font-mono" style={{ color: '#64748b' }}>{t.token.slice(0, 20)}...</code>
            </span>
            <button
              onClick={() => apiRequest(`/api/device-tokens/${t.token}`, { method: 'DELETE' }).then(() => apiRequest('/api/device-tokens').then(r => r.json()).then(setTokens))}
              className="text-xs cursor-pointer flex items-center gap-1 transition-colors"
              style={{ color: '#ef4444' }}
            >
              <Trash2 className="w-3 h-3" />
              Revoke
            </button>
          </div>
        ))}
      </div>
    </div>
  )
}

function SkillsTab() {
  const [skills, setSkills] = useState<Array<{ name: string; description: string; always_on: boolean }>>([])
  const [name, setName] = useState('')
  const [content, setContent] = useState('')

  useEffect(() => { loadSkills() }, [])

  function loadSkills() {
    apiRequest('/api/skills').then(r => r.json()).then(d => setSkills(d.skills || [])).catch(() => {})
  }

  async function createSkill() {
    if (!name.trim() || !content.trim()) return
    await apiRequest('/api/skills', { method: 'POST', body: JSON.stringify({ name, content }) })
    setName(''); setContent('')
    loadSkills()
  }

  async function deleteSkill(n: string) {
    await apiRequest(`/api/skills/${n}`, { method: 'DELETE' })
    loadSkills()
  }

  return (
    <div className="space-y-4">
      <div className="space-y-2">
        <input
          value={name}
          onChange={e => setName(e.target.value)}
          placeholder="Skill name"
          className="w-full px-3 py-2 rounded-xl text-sm text-white placeholder-slate-500 focus:outline-none focus:ring-2 focus:ring-indigo-500/50"
          style={inputStyle}
        />
        <textarea
          value={content}
          onChange={e => setContent(e.target.value)}
          placeholder="SKILL.md content (with frontmatter)"
          rows={6}
          className="w-full px-3 py-2 rounded-xl text-sm font-mono text-white placeholder-slate-500 focus:outline-none focus:ring-2 focus:ring-indigo-500/50"
          style={inputStyle}
        />
        <button
          onClick={createSkill}
          className="px-4 py-2 text-white rounded-xl text-sm font-medium cursor-pointer"
          style={{ background: 'linear-gradient(135deg, #6366f1, #8b5cf6)' }}
        >
          Create Skill
        </button>
      </div>
      {skills.map(s => (
        <div key={s.name} className="flex justify-between items-center py-2 text-sm" style={{ borderTop: '1px solid rgba(255,255,255,0.08)' }}>
          <div>
            <span className="font-medium text-white">{s.name}</span>
            <span className="ml-2" style={{ color: '#64748b' }}>{s.description}</span>
            {s.always_on && (
              <span className="ml-2 text-xs px-1.5 py-0.5 rounded" style={{ background: 'rgba(34, 197, 94, 0.15)', color: '#22c55e' }}>
                always-on
              </span>
            )}
          </div>
          <button onClick={() => deleteSkill(s.name)} className="text-xs cursor-pointer flex items-center gap-1" style={{ color: '#ef4444' }}>
            <Trash2 className="w-3 h-3" />
            Delete
          </button>
        </div>
      ))}
    </div>
  )
}

function SoulTab() {
  const [soul, setSoul] = useState('')
  const [saved, setSaved] = useState(false)

  useEffect(() => {
    apiRequest('/api/user/soul').then(r => r.json()).then(d => setSoul(d.soul || '')).catch(() => {})
  }, [])

  async function save() {
    await apiRequest('/api/user/soul', { method: 'PATCH', body: JSON.stringify({ soul }) })
    setSaved(true); setTimeout(() => setSaved(false), 2000)
  }

  return (
    <div className="space-y-3">
      <p className="text-sm" style={{ color: '#64748b' }}>Define your agent's personality and instructions.</p>
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

function MemoryTab() {
  const [memory, setMemory] = useState('')
  const [saved, setSaved] = useState(false)
  const MAX_CHARS = 4096

  useEffect(() => {
    apiRequest('/api/user/memory').then(r => r.json()).then(d => setMemory(d.memory || '')).catch(() => {})
  }, [])

  async function save() {
    if (memory.length > MAX_CHARS) {
      alert(`Memory exceeds ${MAX_CHARS} character limit`)
      return
    }
    await apiRequest('/api/user/memory', { method: 'PATCH', body: JSON.stringify({ memory }) })
    setSaved(true); setTimeout(() => setSaved(false), 2000)
  }

  return (
    <div className="space-y-3">
      <p className="text-sm" style={{ color: '#64748b' }}>Persistent memory shared across all sessions. The agent can also edit this via save_memory / edit_memory tools.</p>
      <textarea
        value={memory}
        onChange={e => setMemory(e.target.value)}
        rows={12}
        className="w-full px-3 py-2 rounded-xl text-sm font-mono text-white placeholder-slate-500 focus:outline-none focus:ring-2 focus:ring-indigo-500/50"
        style={inputStyle}
      />
      <div className="flex items-center gap-3">
        <button
          onClick={save}
          className="px-4 py-2 text-white rounded-xl text-sm font-medium cursor-pointer"
          style={{ background: 'linear-gradient(135deg, #6366f1, #8b5cf6)', boxShadow: '0 0 20px rgba(99, 102, 241, 0.2)' }}
        >
          Save
        </button>
        <span className={`text-sm ${memory.length > MAX_CHARS ? 'font-medium' : ''}`} style={{ color: memory.length > MAX_CHARS ? '#ef4444' : '#64748b' }}>
          {memory.length} / {MAX_CHARS}
        </span>
        {saved && <span className="text-sm" style={{ color: '#22c55e' }}>Saved!</span>}
      </div>
    </div>
  )
}

function PreferencesTab() {
  const [prefs, setPrefs] = useState('')
  const [saved, setSaved] = useState(false)

  useEffect(() => {
    apiRequest('/api/user/preferences').then(r => r.json()).then(d => setPrefs(JSON.stringify(d.preferences || {}, null, 2))).catch(() => {})
  }, [])

  async function save() {
    try {
      const parsed = JSON.parse(prefs)
      await apiRequest('/api/user/preferences', { method: 'PATCH', body: JSON.stringify({ preferences: parsed }) })
      setSaved(true); setTimeout(() => setSaved(false), 2000)
    } catch { alert('Invalid JSON') }
  }

  return (
    <div className="space-y-3">
      <p className="text-sm" style={{ color: '#64748b' }}>User preferences (JSON format).</p>
      <textarea
        value={prefs}
        onChange={e => setPrefs(e.target.value)}
        rows={10}
        className="w-full px-3 py-2 rounded-xl text-sm font-mono text-white placeholder-slate-500 focus:outline-none focus:ring-2 focus:ring-indigo-500/50"
        style={inputStyle}
      />
      <SaveButton onClick={save} saved={saved} />
    </div>
  )
}

function CronTab() {
  const [jobs, setJobs] = useState<Array<{ job_id: string; name: string; enabled: boolean; cron_expr?: string; every_seconds?: number; next_run_at?: string }>>([])
  const [message, setMessage] = useState('')
  const [cronExpr, setCronExpr] = useState('')

  useEffect(() => { loadJobs() }, [])

  function loadJobs() {
    apiRequest('/api/cron-jobs').then(r => r.json()).then(d => setJobs(Array.isArray(d) ? d : [])).catch(() => {})
  }

  async function createJob() {
    if (!message.trim()) return
    await apiRequest('/api/cron-jobs', {
      method: 'POST',
      body: JSON.stringify({
        message,
        cron_expr: cronExpr || undefined,
        channel: 'gateway',
        chat_id: 'web',
      }),
    })
    setMessage(''); setCronExpr('')
    loadJobs()
  }

  async function deleteJob(id: string) {
    await apiRequest(`/api/cron-jobs/${id}`, { method: 'DELETE' })
    loadJobs()
  }

  async function toggleJob(id: string, enabled: boolean) {
    await apiRequest(`/api/cron-jobs/${id}`, { method: 'PATCH', body: JSON.stringify({ enabled: !enabled }) })
    loadJobs()
  }

  return (
    <div className="space-y-4">
      <div className="space-y-2">
        <input
          value={message}
          onChange={e => setMessage(e.target.value)}
          placeholder="Task message (what the agent should do)"
          className="w-full px-3 py-2 rounded-xl text-sm text-white placeholder-slate-500 focus:outline-none focus:ring-2 focus:ring-indigo-500/50"
          style={inputStyle}
        />
        <input
          value={cronExpr}
          onChange={e => setCronExpr(e.target.value)}
          placeholder="Cron expression (e.g., 0 9 * * *)"
          className="w-full px-3 py-2 rounded-xl text-sm text-white placeholder-slate-500 focus:outline-none focus:ring-2 focus:ring-indigo-500/50"
          style={inputStyle}
        />
        <button
          onClick={createJob}
          className="px-4 py-2 text-white rounded-xl text-sm font-medium cursor-pointer"
          style={{ background: 'linear-gradient(135deg, #6366f1, #8b5cf6)' }}
        >
          Create Job
        </button>
      </div>
      {jobs.map(j => (
        <div key={j.job_id} className="flex justify-between items-center py-2 text-sm" style={{ borderTop: '1px solid rgba(255,255,255,0.08)' }}>
          <div>
            <span className={j.enabled ? 'text-white' : 'line-through'} style={j.enabled ? {} : { color: '#64748b' }}>{j.name}</span>
            <span className="ml-2 text-xs" style={{ color: '#64748b' }}>{j.cron_expr || `every ${j.every_seconds}s`}</span>
          </div>
          <div className="flex gap-3">
            <button
              onClick={() => toggleJob(j.job_id, j.enabled)}
              className="text-xs cursor-pointer flex items-center gap-1 transition-colors"
              style={{ color: j.enabled ? '#f59e0b' : '#22c55e' }}
            >
              {j.enabled ? <PowerOff className="w-3 h-3" /> : <Power className="w-3 h-3" />}
              {j.enabled ? 'Disable' : 'Enable'}
            </button>
            <button
              onClick={() => deleteJob(j.job_id)}
              className="text-xs cursor-pointer flex items-center gap-1"
              style={{ color: '#ef4444' }}
            >
              <Trash2 className="w-3 h-3" />
              Delete
            </button>
          </div>
        </div>
      ))}
    </div>
  )
}
