<script setup lang="ts">
import { ref } from 'vue'
import { useRouter } from 'vue-router'
import { useAuthStore } from '@/stores/auth'

const username = ref('')
const password = ref('')
const role = ref<'admin' | 'user'>('user')
const error = ref('')
const auth = useAuthStore()
const router = useRouter()

const submit = async () => {
  error.value = ''
  try {
    await auth.login({ username: username.value, password: password.value, role: role.value })
    await router.push(role.value === 'admin' ? '/admin' : '/app')
  } catch (e: any) {
    error.value = e?.response?.data?.message || 'login failed'
  }
}
</script>

<template>
  <main class="wrap">
    <h1>登录</h1>
    <select v-model="role">
      <option value="user">用户</option>
      <option value="admin">管理员</option>
    </select>
    <input v-model="username" placeholder="username" />
    <input v-model="password" type="password" placeholder="password" />
    <button @click="submit">登录</button>
    <p v-if="error" class="err">{{ error }}</p>
  </main>
</template>

<style scoped>
.wrap { max-width: 320px; margin: 48px auto; display:flex; flex-direction:column; gap:8px; }
.err { color: #b91c1c; }
</style>
