import { useState, useEffect } from 'react'
import { Link } from 'react-router-dom'
import { apiRequest } from '../lib/api'

type Tab = 'profile' | 'devices' | 'skills' | 'soul' | 'preferences' | 'cron'

export default function SettingsPage() {
  const [tab, setTab] = useState<Tab>('profile')

  const tabs: { id: Tab; label: string }[] = [
    { id: 'profile', label: 'Profile' },
    { id: 'devices', label: 'Devices' },
    { id: 'skills', label: 'Skills' },
    { id: 'soul', label: 'Soul' },
    { id: 'preferences', label: 'Preferences' },
    { id: 'cron', label: 'Cron Jobs' },
  ]

  return (
    <div className="min-h-screen bg-gray-50">
      <div className="max-w-4xl mx-auto py-8 px-4">
        <div className="flex items-center justify-between mb-6">
          <h1 className="text-2xl font-bold">Settings</h1>
          <Link to="/chat" className="text-blue-600 hover:underline text-sm">Back to Chat</Link>
        </div>

        <div className="bg-white rounded-lg shadow">
          <div className="border-b border-gray-200 flex overflow-x-auto">
            {tabs.map(t => (
              <button
                key={t.id}
                onClick={() => setTab(t.id)}
                className={`px-4 py-3 text-sm font-medium whitespace-nowrap ${
                  tab === t.id ? 'border-b-2 border-blue-600 text-blue-600' : 'text-gray-500 hover:text-gray-700'
                }`}
              >
                {t.label}
              </button>
            ))}
          </div>

          <div className="p-6">
            {tab === 'profile' && <ProfileTab />}
            {tab === 'devices' && <DevicesTab />}
            {tab === 'skills' && <SkillsTab />}
            {tab === 'soul' && <SoulTab />}
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

  if (!profile) return <p className="text-gray-500">Loading...</p>

  return (
    <div className="space-y-4">
      <div>
        <label className="block text-sm font-medium text-gray-500">Email</label>
        <p className="text-lg">{profile.email}</p>
      </div>
      <div>
        <label className="block text-sm font-medium text-gray-500">User ID</label>
        <p className="text-sm font-mono text-gray-600">{profile.user_id}</p>
      </div>
      <div>
        <label className="block text-sm font-medium text-gray-500">Role</label>
        <p>{profile.is_admin ? 'Admin' : 'User'}</p>
      </div>
      <div>
        <label className="block text-sm font-medium text-gray-500">Created</label>
        <p>{new Date(profile.created_at).toLocaleDateString()}</p>
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
        <h3 className="font-medium mb-2">Online Devices</h3>
        {devices.length === 0 ? <p className="text-gray-500 text-sm">No devices connected</p> : (
          <table className="w-full text-sm">
            <thead><tr className="text-left text-gray-500"><th className="pb-2">Name</th><th>Tools</th><th>Last Seen</th></tr></thead>
            <tbody>{devices.map(d => (
              <tr key={d.device_name} className="border-t">
                <td className="py-2">{d.device_name}</td>
                <td>{d.tools_count}</td>
                <td>{d.last_seen_secs_ago < 60 ? 'Online' : `${Math.round(d.last_seen_secs_ago / 60)}m ago`}</td>
              </tr>
            ))}</tbody>
          </table>
        )}
      </div>

      <div>
        <h3 className="font-medium mb-2">Device Tokens</h3>
        <div className="flex gap-2 mb-3">
          <input value={newName} onChange={e => setNewName(e.target.value)} placeholder="Device name" className="flex-1 px-3 py-1 border rounded text-sm" />
          <button onClick={createToken} className="px-3 py-1 bg-blue-600 text-white rounded text-sm">Create</button>
        </div>
        {tokens.map(t => (
          <div key={t.token} className="flex justify-between items-center py-1 text-sm border-t">
            <span>{t.device_name} <code className="text-xs text-gray-400">{t.token.slice(0, 20)}...</code></span>
            <button onClick={() => apiRequest(`/api/device-tokens/${t.token}`, { method: 'DELETE' }).then(() => apiRequest('/api/device-tokens').then(r => r.json()).then(setTokens))} className="text-red-500 text-xs">Revoke</button>
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
        <input value={name} onChange={e => setName(e.target.value)} placeholder="Skill name" className="w-full px-3 py-1 border rounded text-sm" />
        <textarea value={content} onChange={e => setContent(e.target.value)} placeholder="SKILL.md content (with frontmatter)" rows={6} className="w-full px-3 py-1 border rounded text-sm font-mono" />
        <button onClick={createSkill} className="px-3 py-1 bg-blue-600 text-white rounded text-sm">Create Skill</button>
      </div>
      {skills.map(s => (
        <div key={s.name} className="flex justify-between items-center py-2 border-t text-sm">
          <div>
            <span className="font-medium">{s.name}</span>
            <span className="text-gray-500 ml-2">{s.description}</span>
            {s.always_on && <span className="ml-2 text-xs bg-green-100 text-green-700 px-1 rounded">always-on</span>}
          </div>
          <button onClick={() => deleteSkill(s.name)} className="text-red-500 text-xs">Delete</button>
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
      <p className="text-sm text-gray-500">Define your agent's personality and instructions.</p>
      <textarea value={soul} onChange={e => setSoul(e.target.value)} rows={10} className="w-full px-3 py-2 border rounded text-sm" />
      <div className="flex items-center gap-3">
        <button onClick={save} className="px-4 py-2 bg-blue-600 text-white rounded text-sm">Save</button>
        {saved && <span className="text-green-600 text-sm">Saved!</span>}
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
      <p className="text-sm text-gray-500">User preferences (JSON format).</p>
      <textarea value={prefs} onChange={e => setPrefs(e.target.value)} rows={10} className="w-full px-3 py-2 border rounded text-sm font-mono" />
      <div className="flex items-center gap-3">
        <button onClick={save} className="px-4 py-2 bg-blue-600 text-white rounded text-sm">Save</button>
        {saved && <span className="text-green-600 text-sm">Saved!</span>}
      </div>
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
        <input value={message} onChange={e => setMessage(e.target.value)} placeholder="Task message (what the agent should do)" className="w-full px-3 py-1 border rounded text-sm" />
        <input value={cronExpr} onChange={e => setCronExpr(e.target.value)} placeholder="Cron expression (e.g., 0 9 * * *)" className="w-full px-3 py-1 border rounded text-sm" />
        <button onClick={createJob} className="px-3 py-1 bg-blue-600 text-white rounded text-sm">Create Job</button>
      </div>
      {jobs.map(j => (
        <div key={j.job_id} className="flex justify-between items-center py-2 border-t text-sm">
          <div>
            <span className={j.enabled ? '' : 'text-gray-400 line-through'}>{j.name}</span>
            <span className="text-gray-400 ml-2 text-xs">{j.cron_expr || `every ${j.every_seconds}s`}</span>
          </div>
          <div className="flex gap-2">
            <button onClick={() => toggleJob(j.job_id, j.enabled)} className="text-xs text-blue-500">{j.enabled ? 'Disable' : 'Enable'}</button>
            <button onClick={() => deleteJob(j.job_id)} className="text-xs text-red-500">Delete</button>
          </div>
        </div>
      ))}
    </div>
  )
}
