/**
 * 职责边界：
 * 1. 定义 /auth, /chat, /settings, /admin 四个核心路由。
 * 2. 实现全局前置守卫 (beforeEach)：未登录拦截、Admin权限拦截。
 */
// TODO: import { createRouter, createWebHistory } from 'vue-router'
// TODO: 配置 routes 数组
// TODO: router.beforeEach((to, from, next) => { ... })