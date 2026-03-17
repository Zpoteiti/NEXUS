import axios from 'axios'

export interface ApiError {
  code: string
  message: string
}

const http = axios.create({
  withCredentials: true
})

let adminBasic = ''

http.interceptors.request.use((config) => {
  if (adminBasic) {
    config.headers.Authorization = adminBasic
  }
  return config
})

http.interceptors.response.use(
  (response) => response,
  (error) => {
    if (error?.response?.status === 401) {
      if (window.location.pathname !== '/login') {
        window.location.href = '/login'
      }
    }
    return Promise.reject(error)
  }
)

export const api = {
  setAdminBasic(username: string, password: string) {
    adminBasic = `Basic ${username}:${password}`
  },
  clearAdminBasic() {
    adminBasic = ''
  },
  async login(username: string, password: string) {
    const { data } = await http.post('/auth/login', { username, password })
    return data as { tenant_id: string; user_id: string; csrf_token: string }
  },
  async logout() {
    await http.post('/auth/logout')
  },
  async adminDashboard(days = 7, tenantId?: string) {
    const { data } = await http.get('/api/admin/dashboard', { params: { days, tenant_id: tenantId } })
    return data as {
      total_users: number
      daily_active_users: number
      usage_by_user: Array<{
        tenant_id: string
        user_id: string
        requests: number
        total_input_tokens: number
        total_output_tokens: number
      }>
      usage_trend: Array<{
        day: string
        requests: number
        total_input_tokens: number
        total_output_tokens: number
      }>
    }
  },
  async userDashboard() {
    const { data } = await http.get('/api/user/dashboard')
    return data
  },
  async userSessions(limit = 20, offset = 0) {
    const { data } = await http.get('/api/user/sessions', { params: { limit, offset } })
    return data as { items: Array<{ session_id: string; title: string }> }
  },
  async sessionMemory(sessionId: string) {
    const { data } = await http.get(`/api/user/sessions/${encodeURIComponent(sessionId)}/memory`)
    return data as {
      items: Array<{ tenant_id: string; user_id: string; session_id: string; content: string }>
    }
  },
  async userUsage(limit = 20, offset = 0) {
    const { data } = await http.get('/api/user/usage', { params: { limit, offset } })
    return data as {
      items: Array<{
        request_id: string
        model: string
        input_tokens: number
        output_tokens: number
      }>
    }
  }
}
