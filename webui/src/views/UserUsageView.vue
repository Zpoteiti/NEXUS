<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { api } from '@/api/client'
import NavBar from '@/components/NavBar.vue'

const items = ref<Array<{ request_id: string; model: string; input_tokens: number; output_tokens: number }>>([])
const loading = ref(true)
const error = ref('')

onMounted(async () => {
  try {
    items.value = (await api.userUsage()).items
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
    <h1>用量明细</h1>
    <p v-if="loading">加载中...</p>
    <p v-else-if="error" class="err">{{ error }}</p>
    <table v-else>
      <thead><tr><th>request_id</th><th>model</th><th>input</th><th>output</th></tr></thead>
      <tbody>
        <tr v-for="row in items" :key="row.request_id">
          <td>{{ row.request_id }}</td>
          <td>{{ row.model }}</td>
          <td>{{ row.input_tokens }}</td>
          <td>{{ row.output_tokens }}</td>
        </tr>
      </tbody>
    </table>
  </main>
</template>

<style scoped>
.page { max-width: 1000px; margin: 16px auto; }
.err { color:#b91c1c; }
table { width:100%; border-collapse: collapse; }
th,td { border:1px solid #ddd; padding:6px; }
</style>
