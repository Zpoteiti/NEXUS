<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { useRoute } from 'vue-router'
import { api } from '@/api/client'
import NavBar from '@/components/NavBar.vue'

const route = useRoute()
const items = ref<Array<{ content: string }>>([])
const loading = ref(true)
const error = ref('')

onMounted(async () => {
  try {
    items.value = (await api.sessionMemory(route.params.sessionId as string)).items
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
    <h1>Session {{ route.params.sessionId }}</h1>
    <p v-if="loading">加载中...</p>
    <p v-else-if="error" class="err">{{ error }}</p>
    <ul v-else>
      <li v-for="(m, idx) in items" :key="idx">{{ m.content }}</li>
    </ul>
    <p v-if="!loading && !error && !items.length">暂无记忆</p>
  </main>
</template>

<style scoped>
.page { max-width: 900px; margin: 16px auto; }
.err { color:#b91c1c; }
</style>
