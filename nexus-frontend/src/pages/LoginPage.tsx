import { useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { login, register } from '../lib/api'
import { useAuthStore, parseJwt } from '../lib/store'
import { Cpu, Eye, EyeOff, Loader2 } from 'lucide-react'

export default function LoginPage() {
  const [isRegister, setIsRegister] = useState(false)
  const [email, setEmail] = useState('')
  const [password, setPassword] = useState('')
  const [showPassword, setShowPassword] = useState(false)
  const [error, setError] = useState('')
  const [loading, setLoading] = useState(false)
  const navigate = useNavigate()
  const setAuth = useAuthStore((s) => s.setAuth)

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault()
    setError('')
    setLoading(true)

    try {
      const data = isRegister
        ? await register(email, password)
        : await login(email, password)

      const token = data.token
      const claims = parseJwt(token)
      setAuth(token, claims?.is_admin ?? false, claims?.sub ?? '')
      navigate('/chat')
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : 'An error occurred')
    } finally {
      setLoading(false)
    }
  }

  return (
    <div className="min-h-screen flex items-center justify-center" style={{ background: 'linear-gradient(180deg, #0f172a 0%, #020617 100%)' }}>
      {/* Ambient glow */}
      <div className="fixed top-1/4 left-1/2 -translate-x-1/2 w-96 h-96 rounded-full opacity-10 blur-3xl pointer-events-none" style={{ background: 'radial-gradient(circle, #6366f1, transparent)' }} />

      <div className="w-full max-w-sm mx-4">
        {/* Logo */}
        <div className="flex flex-col items-center mb-8">
          <div className="w-14 h-14 rounded-2xl flex items-center justify-center mb-4" style={{ background: 'linear-gradient(135deg, #6366f1, #8b5cf6)', boxShadow: '0 0 30px rgba(99, 102, 241, 0.3)' }}>
            <Cpu className="w-7 h-7 text-white" />
          </div>
          <h1 className="text-2xl font-bold tracking-tight text-white">NEXUS</h1>
          <p className="text-sm mt-1" style={{ color: '#94a3b8' }}>
            {isRegister ? 'Create your account' : 'Sign in to continue'}
          </p>
        </div>

        {/* Card */}
        <div className="rounded-2xl p-6" style={{ background: 'rgba(15, 23, 42, 0.8)', border: '1px solid rgba(255, 255, 255, 0.08)', backdropFilter: 'blur(20px)' }}>
          <form onSubmit={handleSubmit} className="space-y-4">
            <div>
              <label className="block text-xs font-medium mb-1.5 uppercase tracking-wider" style={{ color: '#94a3b8' }}>Email</label>
              <input
                type="email"
                value={email}
                onChange={(e) => setEmail(e.target.value)}
                className="w-full px-3.5 py-2.5 rounded-xl text-sm text-white placeholder-slate-500 focus:outline-none focus:ring-2 focus:ring-indigo-500/50"
                style={{ background: 'rgba(255, 255, 255, 0.05)', border: '1px solid rgba(255, 255, 255, 0.08)' }}
                placeholder="you@example.com"
                required
              />
            </div>

            <div>
              <label className="block text-xs font-medium mb-1.5 uppercase tracking-wider" style={{ color: '#94a3b8' }}>Password</label>
              <div className="relative">
                <input
                  type={showPassword ? 'text' : 'password'}
                  value={password}
                  onChange={(e) => setPassword(e.target.value)}
                  className="w-full px-3.5 py-2.5 pr-10 rounded-xl text-sm text-white placeholder-slate-500 focus:outline-none focus:ring-2 focus:ring-indigo-500/50"
                  style={{ background: 'rgba(255, 255, 255, 0.05)', border: '1px solid rgba(255, 255, 255, 0.08)' }}
                  placeholder="Enter password"
                  required
                  minLength={6}
                />
                <button
                  type="button"
                  onClick={() => setShowPassword(!showPassword)}
                  className="absolute right-3 top-1/2 -translate-y-1/2 cursor-pointer"
                  style={{ color: '#64748b' }}
                >
                  {showPassword ? <EyeOff className="w-4 h-4" /> : <Eye className="w-4 h-4" />}
                </button>
              </div>
            </div>

            {error && (
              <div className="text-sm p-3 rounded-xl" style={{ background: 'rgba(239, 68, 68, 0.1)', border: '1px solid rgba(239, 68, 68, 0.2)', color: '#fca5a5' }}>
                {error}
              </div>
            )}

            <button
              type="submit"
              disabled={loading}
              className="w-full py-2.5 rounded-xl text-sm font-medium text-white cursor-pointer disabled:opacity-50 disabled:cursor-not-allowed flex items-center justify-center gap-2"
              style={{ background: 'linear-gradient(135deg, #6366f1, #8b5cf6)', boxShadow: '0 0 20px rgba(99, 102, 241, 0.25)' }}
            >
              {loading && <Loader2 className="w-4 h-4 animate-spin" />}
              {loading ? 'Please wait...' : isRegister ? 'Create Account' : 'Sign In'}
            </button>
          </form>
        </div>

        <p className="text-center text-sm mt-6" style={{ color: '#64748b' }}>
          {isRegister ? 'Already have an account?' : "Don't have an account?"}{' '}
          <button
            onClick={() => { setIsRegister(!isRegister); setError('') }}
            className="cursor-pointer hover:underline"
            style={{ color: '#818cf8' }}
          >
            {isRegister ? 'Sign In' : 'Register'}
          </button>
        </p>
      </div>
    </div>
  )
}
