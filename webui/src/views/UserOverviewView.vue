<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { api } from '@/api/client'
import NavBar from '@/components/NavBar.vue'

const loading = ref(true)
const error = ref('')
const data = ref<any>(null)

onMounted(async () => {
  try {
    data.value = await api.userDashboard()
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
    <h1>我的概览</h1>
    <p v-if="loading">加载中...</p>
    <p v-else-if="error" class="err">{{ error }}</p>
    <section v-else class="cards">
      <article class="card"><h3>Sessions</h3><p>{{ data.sessions }}</p></article>
      <article class="card"><h3>Memories</h3><p>{{ data.memories }}</p></article>
      <article class="card"><h3>Requests</h3><p>{{ data.requests }}</p></article>
      <article class="card"><h3>Input</h3><p>{{ data.total_input_tokens }}</p></article>
      <article class="card"><h3>Output</h3><p>{{ data.total_output_tokens }}</p></article>
    </section>
  </main>
</template>

<style scoped>
.page { max-width: 1000px; margin: 16px auto; }
.cards { display:flex; gap:12px; flex-wrap:wrap; }
.card { border:1px solid #ddd; border-radius:8px; padding:12px; min-width:140px; }
.err { color:#b91c1c; }
</style>
