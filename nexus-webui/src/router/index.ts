/**
 * 职责边界：
 * 1. 定义 /auth、/chat、/settings、/admin 四个核心路由，各自对应的 view 文件如下：
 *    - /auth      → views/AuthView.vue      （登录页，未登录用户的入口）
 *    - /chat      → views/ChatView.vue      （主对话界面，需登录）
 *    - /settings  → views/Settings.vue      （用户设置页，需登录）
 *    - /admin     → views/AdminView.vue     （管理员控制台，需登录 + isAdmin 守卫）
 * 2. 实现全局前置守卫 (beforeEach)，规则如下：
 *    - 未登录用户访问任何需鉴权路由 → 重定向至 /auth
 *    - 已登录用户访问 /auth → 重定向至 /chat（避免重复登录）
 *    - 已登录但 isAdmin 为 false 的用户访问 /admin → 重定向至 /chat（权限不足）
 * 3. isAdmin 状态从 stores/user.ts 读取，守卫在此集中处理，各 View 组件内无需重复鉴权。
 */
// TODO: import { createRouter, createWebHistory } from 'vue-router'
// TODO: 配置 routes 数组（path, component, meta.requiresAuth, meta.requiresAdmin）
// TODO: router.beforeEach((to, from, next) => { ... })
