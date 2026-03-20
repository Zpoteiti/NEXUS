<!--
  职责边界：AdminView（/admin）
  1. 展示管理员控制台：在线设备列表、用户管理、系统日志等运维视图。
  2. 通过 api/rest.ts 调用后端管理接口获取数据。
  3. 此路由受双重守卫保护（router/index.ts 的 beforeEach）：
     - 未登录 → 重定向至 /auth
     - 已登录但 isAdmin 为 false → 重定向至 /chat（403 拒绝进入）
  4. 本组件内不重复鉴权逻辑，权限校验完全由 router 守卫负责。
-->
<template>
  <div></div>
</template>
<script setup lang="ts"></script>
