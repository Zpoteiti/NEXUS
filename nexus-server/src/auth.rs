/// 职责边界：
/// 负责用户认证的完整生命周期：注册、登录、JWT 签发与验证，以及 Axum 中间件封装。
///
/// ─────────────────────────────────────────────────────────────────────────────
/// 【端点】
/// ─────────────────────────────────────────────────────────────────────────────
/// POST /api/auth/register  →  register()
/// POST /api/auth/login     →  login()
///
/// ─────────────────────────────────────────────────────────────────────────────
/// 【注册流程】
/// ─────────────────────────────────────────────────────────────────────────────
/// 1. 接收 RegisterRequest { email, password, admin_token?: Option<String> }
/// 2. 用 bcrypt 对 password 做哈希（cost factor 从 ServerConfig 读取，默认 12）
/// 3. 调用 db::create_user(email, password_hash, is_admin) 写入 users 表
/// 4. Admin 注册判断：若请求体包含 admin_token 字段，且其值与环境变量 ADMIN_TOKEN 一致，
///    则 is_admin = true；字段缺失或值不符不报错，静默注册为普通用户
/// 5. 注册成功后立即签发 JWT，返回 AuthResponse { token, user_id, is_admin }
///
/// ─────────────────────────────────────────────────────────────────────────────
/// 【登录流程】
/// ─────────────────────────────────────────────────────────────────────────────
/// 1. 接收 LoginRequest { email, password }
/// 2. 调用 db::get_user_by_email(email) 取出 password_hash
/// 3. 用 bcrypt::verify(password, hash) 校验密码
/// 4. 校验通过后签发 JWT，Claims 包含 { sub: user_id, is_admin, exp }
/// 5. JWT_SECRET 从 ServerConfig（即环境变量）读取；过期时间建议 7 天
///
/// ─────────────────────────────────────────────────────────────────────────────
/// 【JWT 验证中间件】
/// ─────────────────────────────────────────────────────────────────────────────
/// jwt_middleware() 返回一个 Axum Tower Layer，用法：
///   router.route("/api/sessions", get(handler)).layer(jwt_middleware())
/// 中间件从请求头 Authorization: Bearer <token> 中提取 token，
/// 调用 verify_jwt() 解析 Claims，将 Claims 注入 Axum Extensions 供 handler 读取。
/// WebSocket 握手（ws.rs）中也调用 verify_jwt() 验证 Client 连接时上报的凭据。
///
/// ─────────────────────────────────────────────────────────────────────────────
/// 【参考 nanobot】
/// ─────────────────────────────────────────────────────────────────────────────
/// nanobot 无 JWT，认证委托给各聊天平台的 OAuth/ID 体系
/// （nanobot/channels/base.py  is_allowed() L79-87，allow_from 白名单）。
/// Nexus 需要自己实现多用户隔离，故独立实现完整的注册/登录/JWT 流程。

// TODO: 定义请求/响应结构体
//   pub struct RegisterRequest { pub email: String, pub password: String, pub admin_token: Option<String> }
//   pub struct LoginRequest    { pub email: String, pub password: String }
//   pub struct AuthResponse    { pub token: String, pub user_id: String, pub is_admin: bool }
//   pub struct Claims          { pub sub: String, pub is_admin: bool, pub exp: usize }

// TODO: pub async fn register(
//           payload: RegisterRequest,
//           db: PgPool,
//           admin_token_env: &str,
//       ) -> Result<AuthResponse>
//   注册新用户；admin_token_env 由调用方（api.rs 路由）从 ServerConfig 传入。

// TODO: pub async fn login(payload: LoginRequest, db: PgPool) -> Result<AuthResponse>
//   验证密码，签发 JWT。

// TODO: pub fn verify_jwt(token: &str, secret: &str) -> Result<Claims>
//   解析并验证 JWT 签名与过期时间，返回 Claims。
//   供 jwt_middleware() 内部调用，也供 ws.rs 握手认证直接调用。

// TODO: pub fn jwt_middleware(secret: String) -> impl Layer<...>
//   Axum Tower Layer；从 Authorization 头提取 Bearer token，
//   调用 verify_jwt()，验证失败返回 401，成功将 Claims 插入 Extensions。
