//! MCP 工具定义。
//!
//! 每个子模块是一个独立的 `#[tool_router] impl` 块，在
//! [`crate::mcp::AutoControl::all_tools`] 中合并。工具方法只做三件事：
//! 参数校验、调用 `device` 层、格式化返回。**不要**在工具方法里直接拼报文。

pub mod keyboard;
pub mod mouse;
pub mod system;
