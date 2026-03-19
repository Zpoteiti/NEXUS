/**
 * 职责边界：
 * 1. 存储当前登录用户的 JWT Token、基本信息 (email, name) 和 角色 (role)。
 * 2. 提供 login(), register(), logout() 等 actions，与 server/api.rs 交互。
 * 3. 提供 isAdmin getter 供 UI 组件使用 (例如 v-if="userStore.isAdmin")。
 */
// TODO: import { defineStore } from 'pinia'
// TODO: export const useUserStore = defineStore('user', { ... })