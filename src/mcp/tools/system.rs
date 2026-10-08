//! 状态查询与安全阀。

use rmcp::ErrorData as McpError;
use rmcp::model::CallToolResult;
use rmcp::{tool, tool_router};

use super::super::{AutoControl, render, text, to_mcp_error};
use crate::device::keyboard::is_modifier;

/// 把位掩码还原成按钮名，便于人读。
fn button_names(mask: u8) -> String {
    let mut names = Vec::new();
    if mask & 0x01 != 0 {
        names.push("左键");
    }
    if mask & 0x02 != 0 {
        names.push("右键");
    }
    if mask & 0x04 != 0 {
        names.push("中键");
    }
    if names.is_empty() {
        "无".to_string()
    } else {
        names.join("+")
    }
}

/// 把 HID KeyCode 列表渲染成可读形式。
fn key_names(codes: &[u8]) -> String {
    if codes.is_empty() {
        return "无".to_string();
    }
    codes
        .iter()
        .map(|code| {
            let kind = if is_modifier(*code) {
                "修饰键"
            } else {
                "键"
            };
            format!("{code:#04X}({kind})")
        })
        .collect::<Vec<_>>()
        .join(", ")
}

#[tool_router(router = system_tools, vis = "pub(crate)")]
impl AutoControl {
    #[tool(
        description = "查询服务端与 broker 的连接状态、服务端记录的已按下键/鼠标按钮，\
以及虚拟光标位置。注意：设备固件只订阅、不回报，因此这里**无法**反映设备是否在线，\
也无法反映真实光标坐标。"
    )]
    async fn device_status(&self) -> Result<CallToolResult, McpError> {
        let state = self.devices().state().await;
        let publisher = self.devices().publisher();
        let config = self.devices().config();

        let connected = if publisher.is_connected() {
            "已连接"
        } else {
            "未连接（动作不会下发）"
        };

        Ok(text(format!(
            "MQTT broker：{}（{connected}）\n\
             本端 client id：{}\n\
             鼠标主题：{}　键盘主题：{}\n\
             虚拟光标（估计值）：({}, {})\n\
             服务端按下的鼠标按钮：{}\n\
             服务端按下的键：{}\n\
             帧间隔：{} ms　单帧最大位移：{}\n\
             \n\
             说明：设备固件只订阅、不发布（无 ACK、无状态主题），\
所以「已连接」只表示服务端到 broker 的通路正常，不代表设备收到过报文；\
若设备正在重连（约 10~15 秒窗口），期间的报文会静默丢失。\
服务端按下的键与按钮同样只是服务端自己的记录，设备上的物理按键不在其中。\
若多个实例共用同一 client id，broker 会互相踢掉旧会话，\
表现为「已连接」在每次查询间跳变——client id 带 PID 后缀即可避免。",
            publisher.broker(),
            publisher.client_id(),
            publisher.mouse_topic(),
            publisher.keyboard_topic(),
            state.cursor.0,
            state.cursor.1,
            button_names(state.buttons),
            key_names(&state.keys.iter().copied().collect::<Vec<_>>()),
            config.frame_interval_ms,
            config.max_step,
        )))
    }

    #[tool(description = "急停：释放本服务按下过的所有键与鼠标按钮。\
只解除本服务按下的状态——设备上的物理按键不受影响，已经生效的操作系统侧动作也无法撤销。")]
    async fn emergency_stop(&self) -> Result<CallToolResult, McpError> {
        let report = self
            .devices()
            .emergency_stop()
            .await
            .map_err(to_mcp_error)?;
        let state = self.devices().state().await;
        Ok(text(render(
            "急停",
            report,
            &[
                format!("剩余按下按钮：{}", button_names(state.buttons)),
                format!(
                    "剩余按下键：{}",
                    key_names(&state.keys.iter().copied().collect::<Vec<_>>())
                ),
                "注意：物理按键与已生效的系统动作不在解除范围内。".to_string(),
            ],
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn button_names_are_readable() {
        assert_eq!(button_names(0), "无");
        assert_eq!(button_names(0x01), "左键");
        assert_eq!(button_names(0x03), "左键+右键");
        assert_eq!(button_names(0x07), "左键+右键+中键");
    }

    #[test]
    fn key_names_marks_modifiers() {
        assert_eq!(key_names(&[]), "无");
        assert!(key_names(&[0xE1]).contains("修饰键"));
        assert!(key_names(&[0x04]).contains("(键)"));
    }
}
