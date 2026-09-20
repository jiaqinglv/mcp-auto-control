//! mcp-auto-control：通过 MQTT 驱动 ESP32 自动鼠标/键盘的 MCP 服务端。
//!
//! 分层：
//! - [`config`] —— 命令行与环境变量配置
//! - [`mqtt`] —— 设备报文编解码与 MQTT 发布
//! - [`device`] —— 动作语义（位移拆分、按键映射、虚拟光标、节流）
//! - [`mcp`] —— MCP 工具定义与 handler
//!
//! 拆出 lib target 是为了让 `mqtt` / `device` 里的精确逻辑（字节级报文、位移拆分、
//! 键位表）能在 `tests/` 里直接断言，而不是只测二进制内部函数。

pub mod config;
pub mod device;
pub mod mcp;
pub mod mqtt;
