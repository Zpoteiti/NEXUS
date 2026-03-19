/**
 * 职责边界：
 * 1. 封装所有对 `nexus-server` 的 HTTP REST API 调用 (对应 server 的 api.rs)。
 * 2. 包含：拉取历史会话、在线设备、系统配置等非实时交互数据。
 * 3. 统一处理 HTTP 错误和 Loading 状态。
 */

// TODO: export const fetchSessions = async () => { ... }
// TODO: export const fetchOnlineDevices = async () => { ... }