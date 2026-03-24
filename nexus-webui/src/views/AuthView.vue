<!--
  职责边界：AuthView（/auth）
  1. 【登录】收集用户名/密码，调用 api/rest.ts 完成认证；
     认证成功后将 token 写入 stores/user.ts，router 跳转至 /chat。
  2. 【普通注册】收集用户名/密码完成新账户注册，注册成功后自动登录。
  3. 【Admin 注册】注册表单提供可选的"Admin Token"输入框；
     若用户填写了有效的 Admin Token，服务端将该账户标记为管理员（isAdmin = true）；
     未填写或 Token 无效则注册为普通用户，不报错。
  4. 已登录用户访问 /auth 时，router 的 beforeEach 守卫将其重定向至 /chat。
-->
<template>
  <div></div>
</template>
<script setup lang="ts"></script>
