//! MCP 服务端：把设备动作暴露成工具。
//!
//! 工具方法按域分散在 `tools/` 下的多个 `impl` 块里，每个块用
//! `#[tool_router(router = ...)]` 生成自己的路由，最后在 [`AutoControl::all_tools`]
//! 用 `ToolRouter` 的 `+` 合并 —— 单个 `#[tool_router]` 块无法容纳多个模块。

pub mod tools;

use rmcp::ErrorData as McpError;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::model::{
    CallToolResult, ContentBlock, Implementation, ProtocolVersion, ServerCapabilities, ServerConfig,
};
use rmcp::{ServerHandler, tool_handler};

use crate::device::Devices;

/// MCP 服务端状态。`Clone` 是 rmcp 的要求，内部全部是 `Arc` 共享。
#[derive(Clone)]
pub struct AutoControl {
    devices: Devices,
}

impl AutoControl {
    pub fn new(devices: Devices) -> Self {
        Self { devices }
    }

    pub fn devices(&self) -> &Devices {
        &self.devices
    }

    /// 合并各域的工具路由。
    pub fn all_tools() -> ToolRouter<AutoControl> {
        Self::mouse_tools() + Self::keyboard_tools() + Self::system_tools()
    }
}

#[tool_handler(router = Self::all_tools())]
impl ServerHandler for AutoControl {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::from_build_env())
            .with_protocol_version(ProtocolVersion::V_2024_11_05)
            .with_instructions(INSTRUCTIONS.to_string())
    }
}

/// 客户端初始化时看到的说明。这里写清设备能力边界，避免客户端反复试错。
const INSTRUCTIONS: &str = "\
通过 MQTT 驱动两块 ESP32-S3 USB HID 设备：AutoMouse（主题 mouse/auto）与 AutoKeyboard（主题 keyboard/auto）。

必须知道的限制：
1. 鼠标只有**相对位移**能力。`mouse_move_to`/`mouse_position` 依赖服务端维护的虚拟光标，\
会因操作系统指针加速、屏幕边界以及设备上的物理按键而漂移，坐标只是估计值。
2. 键盘只能发送固件 KEY_MAPPING 表中的按键。不支持 CapsLock、右 Win 键（RGui）、\
小键盘与多媒体键；不在表内的键会直接报错。
3. 文本输入只支持 ASCII。中文等非 ASCII 字符会报错——设备没有输入法通道。
4. 设备只订阅、不回报：没有任何执行确认。工具返回成功只代表报文已交给 MQTT broker，\
不代表桌面已产生对应动作。返回值里的 msg_id 可与设备串口日志对账。
5. 设备离线（WiFi/MQTT 重连）时有约 10~15 秒窗口，期间报文会静默丢失。
6. 自动控制期间不要触碰设备上的物理按键：那会产生服务端看不见的位移和按键。

7. **并行发出的多个工具调用，执行顺序不确定**。服务端会并发派发请求，动作之间靠一把
序列锁互斥，因此每个动作内部（例如一次点击的按下/松开）不会被拆散，
但两个动作谁先谁后没有保证 —— 实测同一批发出的 4 次调用会以不同顺序执行。
需要确定顺序时，请**串行调用**（等上一个工具返回后再发下一个），
不要依赖并行调用的顺序来表达「先移动再点击」。

建议：连续动作之间留出间隔；不确定当前状态时先调用 device_status；\
需要中断时调用 emergency_stop（只能解除本服务按下的键与按钮）。";

/// 把领域层的错误转成 MCP 错误。消息本身已经包含可操作的提示。
pub(crate) fn to_mcp_error(err: anyhow::Error) -> McpError {
    McpError::internal_error(err.to_string(), None)
}

/// 工具的文本结果。
pub(crate) fn text(body: String) -> CallToolResult {
    CallToolResult::success(vec![ContentBlock::text(body)])
}

/// 统一的结果文本：动作名 + 帧数 + msg_id 区间 + 耗时 + 附加信息。
pub(crate) fn render(action: &str, report: crate::device::Report, extras: &[String]) -> String {
    let mut out = format!(
        "{action}：下发 {} 帧，msg_id {}..{}，耗时 {} ms",
        report.frames,
        report.first_msg_id,
        report.last_msg_id,
        report.elapsed.as_millis()
    );
    if report.frames == 0 {
        out = format!("{action}：无需下发报文（状态未变化）");
    }
    for extra in extras {
        out.push('\n');
        out.push_str(extra);
    }
    out
}
