import { create } from 'zustand'

interface AuthState {
  token: string | null
  isAdmin: boolean
  userId: string | null
  setAuth: (token: string, isAdmin: boolean, userId: string) => void
  logout: () => void
}

export const useAuthStore = create<AuthState>((set) => ({
  token: localStorage.getItem('jwt'),
  isAdmin: false,
  userId: null,
  setAuth: (token, isAdmin, userId) => {
    localStorage.setItem('jwt', token)
    set({ token, isAdmin, userId })
  },
  logout: () => {
    localStorage.removeItem('jwt')
    set({ token: null, isAdmin: false, userId: null })
  },
}))

// Parse JWT to extract claims (without verification — server validates)
export function parseJwt(token: string): { sub: string; is_admin: boolean; exp: number } | null {
  try {
    const base64 = token.split('.')[1]
    const json = atob(base64)
    return JSON.parse(json)
  } catch {
    return null
  }
}
