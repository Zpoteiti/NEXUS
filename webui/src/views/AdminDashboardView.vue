<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { api } from '@/api/client'
import NavBar from '@/components/NavBar.vue'

const loading = ref(true)
const error = ref('')
const days = ref(7)
const tenantId = ref('')
const data = ref<any>(null)

const load = async () => {
  loading.value = true
  error.value = ''
  try {
    data.value = await api.adminDashboard(days.value, tenantId.value || undefined)
  } catch (e: any) {
    error.value = e?.response?.data?.message || '加载失败'
  } finally {
    loading.value = false
  }
}

onMounted(load)
</script>

<template>
  <NavBar />
  <main class="page">
    <h1>管理员看板</h1>
    <div class="filters">
      <label>天数:
        <select v-model.number="days" @change="load">
          <option :value="7">7</option>
          <option :value="30">30</option>
        </select>
      </label>
      <input v-model="tenantId" placeholder="tenant 筛选(可选)" />
      <button @click="load">查询</button>
    </div>
    <p v-if="loading">加载中...</p>
    <p v-else-if="error" class="err">{{ error }}</p>
    <template v-else>
      <section class="cards">
        <article class="card"><h3>总用户</h3><p>{{ data.total_users }}</p></article>
        <article class="card"><h3>日活</h3><p>{{ data.daily_active_users }}</p></article>
      </section>
      <h2>趋势</h2>
      <table>
        <thead><tr><th>day</th><th>requests</th><th>in</th><th>out</th></tr></thead>
        <tbody>
          <tr v-for="row in data.usage_trend" :key="row.day">
            <td>{{ row.day }}</td><td>{{ row.requests }}</td><td>{{ row.total_input_tokens }}</td><td>{{ row.total_output_tokens }}</td>
          </tr>
        </tbody>
      </table>
      <h2>每用户用量</h2>
      <table>
        <thead><tr><th>tenant</th><th>user</th><th>req</th><th>in</th><th>out</th></tr></thead>
        <tbody>
          <tr v-for="row in data.usage_by_user" :key="`${row.tenant_id}:${row.user_id}`">
            <td>{{ row.tenant_id }}</td><td>{{ row.user_id }}</td><td>{{ row.requests }}</td><td>{{ row.total_input_tokens }}</td><td>{{ row.total_output_tokens }}</td>
          </tr>
        </tbody>
      </table>
    </template>
  </main>
</template>

<style scoped>
.page { max-width: 1000px; margin: 16px auto; }
.filters { display:flex; gap:8px; margin-bottom:12px; }
.cards { display:flex; gap:12px; }
.card { border:1px solid #ddd; border-radius:8px; padding:12px; min-width:150px; }
.err { color:#b91c1c; }
table { width:100%; border-collapse: collapse; margin:12px 0; }
th,td { border:1px solid #ddd; padding:6px; }
</style>
