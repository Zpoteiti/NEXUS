<!--
  职责边界：ChatView（/chat）
  1. 渲染主对话界面，展示历史消息列表（MessageItem 组件）。
  2. 提供用户输入框，捕获提交事件并将消息推入 app store。
  3. 通过 stores/app.ts 订阅 WebSocket 消息流，实现实时消息更新。
  4. 非登录用户会被 router 的 beforeEach 守卫重定向至 /auth，无需在此处重复鉴权。

  布局面板：
  5. 左侧抽屉面板：历史 session 列表，可点击切换会话。
     数据通过 api/rest.ts 的 fetchSessions() 拉取，
     当前激活的 session_id 写入 stores/app.ts 供全局消费。
  6. 右上角状态面板：展示当前登录用户名下所有设备的在线状态
     （设备名、last_seen、在线/离线标记），
     以及每台在线设备已挂载的工具列表（内置工具 + MCP 工具 + Skill 工具）。
     数据通过 api/rest.ts 的 fetchOnlineDevices() 拉取。
-->
<template>
  <div></div>
</template>
<script setup lang="ts"></script>
