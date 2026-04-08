function handleUnauthorized(res: Response): Response {
  if (res.status === 401) {
    localStorage.removeItem('jwt')
    window.location.href = '/login'
    throw new Error('Unauthorized')
  }
  return res
}

export async function apiRequest(path: string, options?: RequestInit) {
  const token = localStorage.getItem('jwt')
  const res = await fetch(path, {
    ...options,
    headers: {
      'Content-Type': 'application/json',
      ...(token ? { Authorization: `Bearer ${token}` } : {}),
      ...options?.headers,
    },
  })
  return handleUnauthorized(res)
}

export async function login(email: string, password: string) {
  const res = await fetch('/api/auth/login', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ email, password }),
  })
  if (!res.ok) {
    const data = await res.json().catch(() => ({ message: 'Login failed' }))
    throw new Error(data.message || 'Login failed')
  }
  return res.json()
}

export async function register(email: string, password: string, adminToken?: string) {
  const body: Record<string, string> = { email, password }
  if (adminToken) body.admin_token = adminToken
  const res = await fetch('/api/auth/register', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
  })
  if (!res.ok) {
    const data = await res.json().catch(() => ({ message: 'Registration failed' }))
    throw new Error(data.message || 'Registration failed')
  }
  return res.json()
}

export async function uploadFile(file: File) {
  const token = localStorage.getItem('jwt')
  const formData = new FormData()
  formData.append('file', file)
  const res = handleUnauthorized(await fetch('/api/files', {
    method: 'POST',
    headers: token ? { Authorization: `Bearer ${token}` } : {},
    body: formData,
  }))
  if (!res.ok) throw new Error('File upload failed')
  return res.json()
}
