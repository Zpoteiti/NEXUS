<!--
  职责边界：AdminView（/admin）
  1. 【设备管理】展示当前所有在线设备列表（设备名、所属用户、最后心跳时间、已注册工具数）。
  2. 【用户管理】列出全部用户账户，支持封禁/解封操作。
  3. 【统计看板】展示 DAU（日活跃用户数）趋势图及 Token 消耗统计图表（按用户/按日期聚合）。
  4. 【全局 LLM 配置】提供全局 LLM 设置表单：模型名称、API Key、endpoint 地址，
     修改后通过 api/rest.ts 持久化到服务端配置。
  5. 【系统日志】展示最近的系统级日志流。
  6. 此路由受双重守卫保护（router/index.ts 的 beforeEach）：
     - 未登录 → 重定向至 /auth
     - 已登录但 isAdmin 为 false → 重定向至 /chat（403 拒绝进入）
  7. 本组件内不重复鉴权逻辑，权限校验完全由 router 守卫负责。
-->
<template>
  <div></div>
</template>
<script setup lang="ts"></script>
