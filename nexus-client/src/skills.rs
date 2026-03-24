/// 职责边界：本地 Skill 目录扫描、.md 解析、Schema 生成、热重载检测、执行时参数传递。
///
/// ─────────────────────────────────────────────────────────────────────────────
/// 【参考：nanobot 实现】
/// ─────────────────────────────────────────────────────────────────────────────
///
/// 1. 目录结构（nanobot/nanobot/skills/）
///    每个 skill 是一个子目录，内含：
///      skills/{name}/SKILL.md          ← 唯一必需文件
///      skills/{name}/scripts/          ← 可选，脚本文件（.py/.sh 等）
///      skills/{name}/references/       ← 可选，按需加载的参考文档
///      skills/{name}/assets/           ← 可选，模板/图标等输出资源
///
/// 2. SKILL.md 格式（nanobot/nanobot/agent/skills.py，`get_skill_metadata()` L203-228）
///    文件头部为 YAML frontmatter，主体为 Markdown 说明文本：
///
///    ---
///    name: <skill-name>
///    description: <一句话描述>
///    always: true          # 可选，true 表示始终注入 system prompt
///    metadata: {"nanobot":{"emoji":"🧵","os":["darwin","linux"],...}}
///    ---
///
///    解析函数：
///      `get_skill_metadata()`   → 提取 YAML frontmatter（skills.py L203-228）
///      `_strip_frontmatter()`   → 用正则 r"^---\n.*?\n---\n" 剥离 frontmatter（L161-167）
///      `_parse_nanobot_metadata()` → 将 metadata 字段反序列化为 JSON（L169-175）
///
/// 3. 热重载（nanobot/nanobot/agent/skills.py，SkillsLoader.__init__() L21-24）
///    nanobot 不支持热重载：SkillsLoader 在 AgentLoop 启动时实例化一次，
///    此后不监听文件系统变化，用户需 /restart 才能生效。
///    对比：cron/service.py 通过 `st_mtime` 实现热重载，skill 侧无此逻辑。
///
///    Nexus 中的设计决策：
///    ┌──────────────────────────────────────────────────────────────────────┐
///    │ 可选择在首次连接时一次性扫描，或通过 notify/inotify 监听目录变更，  │
///    │ 检测到变更后重新生成 Schema 并通过 WebSocket 发送 RegisterTools。   │
///    └──────────────────────────────────────────────────────────────────────┘
///
/// 4. Schema 生成（nanobot/nanobot/agent/tools/base.py，`Tool.to_schema()` L190-199）
///    nanobot 中 skill 不转换为 Tool Schema；skill 以 XML summary 注入 system prompt，
///    LLM 通过 read_file 工具读取 SKILL.md 后自行决策。
///    实际 Tool Schema 格式（OpenAI function calling）：
///      { "type": "function",
///        "function": { "name": ..., "description": ..., "parameters": {...} } }
///
///    Nexus 中的设计决策：
///    ┌──────────────────────────────────────────────────────────────────────┐
///    │ Nexus skill 应当转换为标准 Tool Schema（nexus-common ToolSchema），  │
///    │ 与 MCP 工具、内置工具统一聚合后通过 RegisterTools 发送给 Server，   │
///    │ 使 LLM 可以直接以 function call 方式调用 skill，而非依赖 read_file。│
///    └──────────────────────────────────────────────────────────────────────┘
///
///    参考文件：
///      nanobot/nanobot/agent/context.py  → `ContextBuilder.build_skills_summary()`，
///                                          生成 <skills>...<skill>...</skill></skills> XML
///      nanobot/nanobot/agent/tools/registry.py → `ToolRegistry.get_definitions()` L34-36
///
/// 5. 执行时参数传递（nanobot/nanobot/agent/loop.py L224-231）
///    nanobot 无自动参数绑定：LLM 读取脚本后自行拼接 shell 命令，
///    再通过 exec 工具执行（参数随命令字符串传入）。
///    参数校验仅针对 Tool，见 base.py `cast_params()` / `validate_params()`。
///
///    Nexus 中的设计决策：
///    ┌──────────────────────────────────────────────────────────────────────┐
///    │ 当 Server 下发 ExecuteToolRequest 时，executor.rs 按 skill 名称     │
///    │ 路由到本模块；本模块将 tool call arguments（JSON）按 frontmatter 中 │
///    │ 声明的参数 schema 校验后，以环境变量或命令行参数形式传递给脚本，    │
///    │ 再通过 process.rs 启动子进程执行，捕获 stdout/stderr 回传结果。     │
///    └──────────────────────────────────────────────────────────────────────┘
///
/// ─────────────────────────────────────────────────────────────────────────────
/// 【待实现的函数签名（TODO）】
/// ─────────────────────────────────────────────────────────────────────────────

// TODO: 定义 SkillMeta 结构体，对应 SKILL.md frontmatter 字段
//   struct SkillMeta {
//       name: String,
//       description: String,
//       always: bool,
//       // parameters: Option<serde_json::Value>,  // 扩展：声明式参数 schema
//   }

// TODO: 定义 Skill 结构体，持有元数据 + 根目录路径 + 脚本路径列表
//   struct Skill {
//       meta: SkillMeta,
//       root: PathBuf,              // skills/{name}/
//       script: Option<PathBuf>,    // scripts/ 下的入口脚本
//   }

// TODO: 实现 scan_skills(skills_dir: &Path) -> anyhow::Result<Vec<Skill>>
//   扫描指定目录，遍历每个子目录，读取 SKILL.md，解析 frontmatter，返回 Skill 列表。
//   对应 nanobot: SkillsLoader.list_skills()（skills.py L26-57）

// TODO: 实现 parse_skill_md(path: &Path) -> anyhow::Result<SkillMeta>
//   解析单个 SKILL.md 文件的 YAML frontmatter。
//   对应 nanobot: get_skill_metadata()（skills.py L203-228）

// TODO: 实现 skill_to_schema(skill: &Skill) -> serde_json::Value
//   将 Skill 转换为裸 JSON（OpenAI function calling 格式），
//   与内置工具、MCP 工具保持一致（均为 serde_json::Value），
//   供 discovery.rs 聚合后经 session.rs 注册给 Server（RegisterTools.schemas: Vec<Value>）。
//   对应 nanobot: Tool.to_schema()（tools/base.py L190-199），但 nanobot skill 未做此转换。

// TODO: 实现 execute_skill(skill: &Skill, args: serde_json::Value) -> anyhow::Result<String>
//   按 Tool call arguments 校验参数，构造子进程调用（委托 process.rs），
//   捕获 stdout/stderr，返回执行结果字符串。
//   对应 nanobot: exec tool 调用链（agent/loop.py L224-231 + tools/base.py cast_params）

// TODO: 实现热重载检测（可选）
//   struct SkillWatcher { last_mtime: HashMap<PathBuf, SystemTime>, ... }
//   fn poll_changes(&mut self, skills_dir: &Path) -> Vec<SkillChangeEvent>
//   对应 nanobot cron/service.py 的 st_mtime 模式；skill 侧在 nanobot 中未实现，Nexus 可扩展。
