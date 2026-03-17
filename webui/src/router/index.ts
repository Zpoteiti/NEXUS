import { createRouter, createWebHistory } from 'vue-router'
import { useAuthStore } from '@/stores/auth'
import LoginView from '@/views/LoginView.vue'
import AdminDashboardView from '@/views/AdminDashboardView.vue'
import UserOverviewView from '@/views/UserOverviewView.vue'
import UserSessionsView from '@/views/UserSessionsView.vue'
import SessionDetailView from '@/views/SessionDetailView.vue'
import UserUsageView from '@/views/UserUsageView.vue'

export const router = createRouter({
  history: createWebHistory(),
  routes: [
    { path: '/', redirect: '/app' },
    { path: '/login', component: LoginView },
    { path: '/admin', component: AdminDashboardView, meta: { role: 'admin' } },
    { path: '/app', component: UserOverviewView, meta: { role: 'user' } },
    { path: '/app/sessions', component: UserSessionsView, meta: { role: 'user' } },
    { path: '/app/sessions/:sessionId', component: SessionDetailView, meta: { role: 'user' } },
    { path: '/app/usage', component: UserUsageView, meta: { role: 'user' } }
  ]
})

router.beforeEach(async (to) => {
  const auth = useAuthStore()
  if (!auth.ready) {
    await auth.bootstrap()
  }
  const role = to.meta.role as 'admin' | 'user' | undefined
  if (!role) return true
  if (!auth.isAuthenticated) {
    return '/login'
  }
  if (role === 'admin' && auth.role !== 'admin') return '/app'
  if (role === 'user' && auth.role !== 'user') return '/login'
  return true
})
