<!--
  职责边界：ChatView（/chat）
  1. 渲染主对话界面，展示历史消息列表（MessageItem 组件）。
  2. 提供用户输入框，捕获提交事件并将消息推入 app store。
  3. 通过 stores/app.ts 订阅 WebSocket 消息流，实现实时消息更新。
  4. 非登录用户会被 router 的 beforeEach 守卫重定向至 /auth，无需在此处重复鉴权。
-->
<template>
  <div></div>
</template>
<script setup lang="ts"></script>
