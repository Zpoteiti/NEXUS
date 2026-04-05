import { create } from 'zustand'

interface AuthState {
  token: string | null
  isAdmin: boolean
  userId: string | null
  setAuth: (token: string, isAdmin: boolean, userId: string) => void
  logout: () => void
}

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

function rehydrateFromJwt(): { isAdmin: boolean; userId: string | null } {
  const token = localStorage.getItem('jwt')
  if (!token) return { isAdmin: false, userId: null }
  const claims = parseJwt(token)
  if (!claims) return { isAdmin: false, userId: null }
  // Check if token is expired
  if (claims.exp * 1000 < Date.now()) {
    localStorage.removeItem('jwt')
    return { isAdmin: false, userId: null }
  }
  return { isAdmin: claims.is_admin, userId: claims.sub }
}

export const useAuthStore = create<AuthState>((set) => {
  const { isAdmin, userId } = rehydrateFromJwt()
  return {
    token: localStorage.getItem('jwt'),
    isAdmin,
    userId,
    setAuth: (token, isAdmin, userId) => {
      localStorage.setItem('jwt', token)
      set({ token, isAdmin, userId })
    },
    logout: () => {
      localStorage.removeItem('jwt')
      set({ token: null, isAdmin: false, userId: null })
    },
  }
})
