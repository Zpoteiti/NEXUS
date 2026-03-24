# nexus-webui

## 1. 一句话定位
nexus-webui 是 NEXUS 的浏览器交互层，负责用户登录、会话展示与工具调用过程可视化。

## 2. 职责边界
### 负责什么
- 提供登录、聊天、设置、管理后台四类页面。
- 通过 REST 获取列表型数据，通过聊天 WebSocket 获取实时消息与状态流。
- 在前端本地持有 Device Token，并为请求附带认证信息。
- 在路由层集中执行鉴权与权限守卫。

### 不负责什么
- 不直接连接任何 Client 设备。
- 不执行工具、不做 Agent 编排。
- 不维护协议定义与服务端状态持久化。

## 3. 架构决策（What + Why）
### 决策 A：仅与 Server 通信
- What：WebUI 的所有请求都发往 Server，不与 Client 建立任何直连通道。
- Why：保持网络拓扑单中心，简化安全边界与权限控制。

### 决策 B：双连接分工明确
- What：REST（`/api/*`）承载查询与配置；WebSocket（`/ws/chat`）承载实时对话与状态流。
- Why：将实时通道与 CRUD 通道分离，降低耦合并提升可维护性。

### 决策 C：统一使用 Device Token 认证
- What：登录后获取 Device Token，存储于 `localStorage`，后续请求统一携带该凭据。
- Why：与服务端和设备端认证模型保持一致，减少多套认证逻辑带来的状态分叉。

### 决策 D：技术栈固定为 Vue 3 + Pinia + Vue Router 4 + Element Plus + Vite 5 + TypeScript
- What：UI 组件体系采用 Element Plus，不引入额外样式体系。
- Why：统一组件风格与交互规范，降低样式分裂与维护成本。

### 决策 E：路由守卫集中在 `router/index.ts`
- What：登录态校验、管理员权限校验在全局前置守卫中统一处理。
- Why：避免在每个页面重复实现鉴权逻辑，减少漏检风险。

## 4. 与其他模块的关系
### 依赖谁
- 依赖 `nexus-server` 暴露的 `/api/*` 与 `/ws/chat`。

### 被谁依赖
- 被最终用户通过浏览器访问。

### 通信方式
- WebUI ↔ Server：HTTP REST + WebSocket 聊天流。
- WebUI ↔ Client：无直接通信。

## 5. 环境要求与运行方式
### 环境要求
- Node.js 20+
- 包管理器：npm / pnpm / yarn 任一
- 前端技术栈：Vue 3、Pinia、Vue Router 4、Element Plus、Vite 5、TypeScript

### 运行方式
- 安装依赖后启动 Vite 开发服务。
- 页面登录成功后获取并持久化 Device Token。
- 聊天页建立 `/ws/chat` 连接，实时显示对话与工具调用过程。
