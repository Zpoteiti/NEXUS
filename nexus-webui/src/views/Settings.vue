<!--
  职责边界：SettingsView（/settings）
  1. 【Agent 记忆管理】展示 Agent 对当前用户积累的记忆条目列表，
     用户可逐条查看并删除不需要的记忆，删除操作通过 api/rest.ts 同步到服务端。
  2. 【user_preferences 配置】允许用户配置 Agent 的回复风格偏好（语气、回复详细程度、语言等），
     保存后通过 api/rest.ts 持久化，agent_loop 每次运行时会将 user_preferences 注入 System Prompt。
  3. 【Admin 入口】若当前用户 isAdmin 为 true，展示"进入管理后台"按钮，
     点击后由 router 跳转至 /admin；普通用户不可见此按钮。
  4. 通过 stores/user.ts 读取当前用户信息和 user_preferences。
  5. 非登录用户会被 router 的 beforeEach 守卫重定向至 /auth，无需在此处重复鉴权。
-->
<template>
  <div></div>
</template>
<script setup lang="ts"></script>
