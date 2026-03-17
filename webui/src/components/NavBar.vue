<script setup lang="ts">
import { useAuthStore } from '@/stores/auth'
import { useRouter } from 'vue-router'

const auth = useAuthStore()
const router = useRouter()

const onLogout = async () => {
  await auth.logout()
  await router.push('/login')
}
</script>

<template>
  <header class="nav">
    <strong>NEXUS WebUI</strong>
    <nav>
      <router-link v-if="auth.role === 'admin'" to="/admin">Admin</router-link>
      <template v-if="auth.role === 'user'">
        <router-link to="/app">Overview</router-link>
        <router-link to="/app/sessions">Sessions</router-link>
        <router-link to="/app/usage">Usage</router-link>
      </template>
      <button @click="onLogout">Logout</button>
    </nav>
  </header>
</template>

<style scoped>
.nav { display:flex; justify-content:space-between; padding:12px; border-bottom:1px solid #ddd; }
nav { display:flex; gap:12px; align-items:center; }
</style>
