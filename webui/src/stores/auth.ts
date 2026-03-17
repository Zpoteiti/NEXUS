import { defineStore } from 'pinia'
import { api } from '@/api/client'

interface LoginPayload {
  username: string
  password: string
  role: 'admin' | 'user'
}

export const useAuthStore = defineStore('auth', {
  state: () => ({
    ready: false,
    isAuthenticated: false,
    role: null as null | 'admin' | 'user',
    csrfToken: ''
  }),
  actions: {
    async bootstrap() {
      this.ready = true
    },
    async login(payload: LoginPayload) {
      if (payload.role === 'admin') {
        this.role = 'admin'
        this.isAuthenticated = true
        api.setAdminBasic(payload.username, payload.password)
        return
      }
      const response = await api.login(payload.username, payload.password)
      this.csrfToken = response.csrf_token
      this.role = 'user'
      this.isAuthenticated = true
    },
    async logout() {
      if (this.role === 'user') {
        await api.logout()
      }
      this.role = null
      this.isAuthenticated = false
      this.csrfToken = ''
      api.clearAdminBasic()
    }
  }
})
