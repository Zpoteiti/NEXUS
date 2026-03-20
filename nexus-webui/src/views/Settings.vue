<!--
  职责边界：SettingsView（/settings）
  1. 展示并允许用户编辑个人偏好设置（主题、语言、通知等）。
  2. 通过 stores/user.ts 读取当前用户配置，提交修改后调用 api/rest.ts 持久化到服务端。
  3. 非登录用户会被 router 的 beforeEach 守卫重定向至 /auth，无需在此处重复鉴权。
-->
<template>
  <div></div>
</template>
<script setup lang="ts"></script>
