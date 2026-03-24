/// 职责边界：
/// 1. 仅存放大模型可以使用的 Tools 的 Schema（JSON 描述）。
/// 2. 注意：Server 不执行工具！它只负责把这些 Schema 喂给 LLM，让 LLM 知道 Client 能干什么。
///
/// 参考 nanobot：
/// - 对应 `nanobot/agent/tools/registry.py`，但抛弃了它的 `execute` 方法。
/// - 提取 nanobot 中如 `shell.py`, `read_file.py` 的 JSON Schema 描述放在这里。

// TODO: 实现 get_available_tool_schemas(device_id) -> Vec<Value>