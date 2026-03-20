<!--
  职责边界：AuthView（/auth）
  1. 提供登录入口，收集用户名/密码（或 OAuth 授权码），调用 api/rest.ts 完成认证。
  2. 认证成功后将 token 写入 stores/user.ts，并由 router 跳转至 /chat。
  3. 已登录用户访问 /auth 时，router 的 beforeEach 守卫应将其重定向至 /chat。
-->
<template>
  <div></div>
</template>
<script setup lang="ts"></script>
