<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { api } from '@/api/client'
import NavBar from '@/components/NavBar.vue'

const items = ref<Array<{ session_id: string; title: string }>>([])
const loading = ref(true)
const error = ref('')

onMounted(async () => {
  try {
    items.value = (await api.userSessions()).items
  } catch (e: any) {
    error.value = e?.response?.data?.message || '加载失败'
  } finally {
    loading.value = false
  }
})
</script>

<template>
  <NavBar />
  <main class="page">
    <h1>Sessions</h1>
    <p v-if="loading">加载中...</p>
    <p v-else-if="error" class="err">{{ error }}</p>
    <ul v-else>
      <li v-for="s in items" :key="s.session_id">
        <router-link :to="`/app/sessions/${s.session_id}`">{{ s.title }} ({{ s.session_id }})</router-link>
      </li>
    </ul>
    <p v-if="!loading && !error && !items.length">暂无 session</p>
  </main>
</template>

<style scoped>
.page { max-width: 900px; margin: 16px auto; }
.err { color:#b91c1c; }
</style>
