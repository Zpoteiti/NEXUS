# NEXUS 验收标准（可操作、可复现）

## M0 — 文档基建
1. 在仓库根目录执行：  
   `ls nexus-common/README.md nexus-server/README.md nexus-client/README.md nexus-webui/README.md`  
   预期：四个文件都存在。  
2. 打开四个 README，逐一确认都包含以下章节标题：  
   `一句话定位 / 职责边界 / 架构决策 / 与其他模块的关系 / 环境要求与运行方式`。  
3. 在仓库执行：  
   `rg -n "Device Token|nexus_dev_" nexus-common nexus-server nexus-client nexus-webui`  
   预期：四个模块文档均出现 Device Token 相关描述。  

## M1 — 握手能跑通
1. 前置检查 `.env`：  
   打开 `nexus-server/.env`，肉眼确认只保留 Device Token 方案相关配置项，不存在旧认证方案的遗留变量。  
2. 前置检查数据库表：  
   `psql "$DATABASE_URL" -c "\dt"`  
   预期：最小验收仅需 `users`、`device_tokens` 两张表。  
3. 启动 Server：  
   `docker compose up -d`（在 `nexus-server` 对应部署目录执行）  
   预期：服务监听端口成功，日志可查看。  
4. 启动 Client：  
   `cargo run -p nexus-client`  
   预期：终端出现 `LoginSuccess`。  
5. 验证认证流程：  
   使用一个有效 Device Token 启动 Client 可成功上线；替换为无效 token 后应出现登录失败并断开连接。  

## M2 — 工具注册能跑通
1. 启动 Client 后查看 Server 日志：  
   预期：出现工具注册事件，包含 `device_id` 与工具数量。  
2. 通过服务端状态查询接口或调试日志查看在线设备快照：  
   预期：该设备 `tools` 字段非空且可见 schema。  
3. 修改本地工具集后等待一个心跳周期：  
   预期：Server 端工具快照更新，工具数量或名称与新状态一致。  

## M3 — Agent Loop 跑通
1. 用命令触发一次会话请求：  
   `curl -X POST http://127.0.0.1:8080/api/sessions/<session_id>/messages -H "Content-Type: application/json" -d "{\"content\":\"请执行一次 shell 并返回当前目录\"}"`  
   预期：请求成功返回消息受理结果。  
2. 观察 Server 日志顺序：  
   预期：依次出现“模型调用 → 下发 ExecuteToolRequest → 收到 ToolExecutionResult → 最终回复”。  
3. 人工断开 Client 网络后重试工具调用：  
   预期：挂起请求迅速失败返回，Agent Loop 不会长时间卡住。  
4. 制造参数错误请求：  
   预期：返回退出码 `-3` 且含明确校验失败信息。  

## M4 — WebUI 能用
1. 启动前端：  
   `npm run dev`（在 `nexus-webui` 目录）  
   预期：浏览器可访问本地开发地址。  
2. 登录验证：  
   在登录页输入邮箱与密码，登录后进入聊天页。  
   预期：`localStorage` 中可见 Device Token。  
3. 聊天验证：  
   发送一条会触发工具的消息。  
   预期：页面实时显示工具调用过程与最终回复。  
4. 路由守卫验证：  
   未登录直访 `/chat` 应跳转 `/auth`；非管理员访问 `/admin` 应跳转 `/chat`。  

## M5 — 完善与扩展
1. 记忆系统：  
   触发多轮对话后执行  
   `psql "$DATABASE_URL" -c "select count(*) from memory_chunks;"`  
   预期：计数大于 0。  
2. 向量检索：  
   发起与历史主题相关问题。  
   预期：回复中体现已召回的长期记忆事实。  
3. MCP 工具：  
   挂载一个 MCP Server 后发起对应工具请求。  
   预期：可见 `mcp_` 前缀工具被调用并返回结果。  
4. Skills：  
   在 skills 目录新增一个 Skill 后重连 Client。  
   预期：新 `skill_` 工具出现在可用工具列表并可执行。  
5. 管理后台与渠道：  
   管理员进入后台可见设备与用户信息；Telegram 渠道收发消息可触发 Agent 回复。  
