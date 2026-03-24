/**
 * 职责边界：
 * 1. 管理整个应用的核心全局状态。
 * 2. 记录当前在线的设备列表 (Devices) 和已挂载的 MCP 工具 (Tools)。
 * 3. 记录当前的会话 ID 和聊天消息列表，供视图层渲染。
 */

// import { defineStore } from 'pinia'
// TODO: export const useAppStore = defineStore('app', { state, actions ... })