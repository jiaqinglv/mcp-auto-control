//! 键盘相关工具。

use rmcp::ErrorData as McpError;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use rmcp::{tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;

use super::super::{AutoControl, render, text, to_mcp_error};

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum KeyAction {
    /// 按下并保持
    Press,
    /// 松开
    Release,
    /// 按下后立即松开
    Tap,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct KeyArgs {
    /// 键名，如 a、1、enter、esc、tab、space、backspace、delete、up、down、left、right、
    /// home、end、pageup、pagedown、insert、f1-f12、ctrl、shift、alt、gui、
    /// right_ctrl、right_shift、right_alt
    pub key: String,
    /// 动作，默认 tap
    pub action: Option<KeyAction>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ComboArgs {
    /// 要同时按下的键名列表，最后一个是主键。例如 ["ctrl", "shift", "s"] 表示 Ctrl+Shift+S
    pub keys: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TextArgs {
    /// 要输入的文本，仅支持 ASCII 可见字符与 \n \t
    pub text: String,
}

#[tool_router(router = keyboard_tools, vis = "pub(crate)")]
impl AutoControl {
    #[tool(
        description = "按键：press 按下并保持、release 松开、tap 按下后立即松开（默认）。\
键名必须是设备固件键位表中的键，例如 a-z、0-9、enter、esc、tab、space、backspace、\
delete、insert、home、end、pageup、pagedown、up/down/left/right、f1-f12、\
ctrl、shift、alt、gui、right_ctrl、right_shift、right_alt。\
设备不支持 CapsLock、右 Win 键（RGui）、小键盘与多媒体键。"
    )]
    async fn keyboard_key(
        &self,
        Parameters(KeyArgs { key, action }): Parameters<KeyArgs>,
    ) -> Result<CallToolResult, McpError> {
        let action = action.unwrap_or(KeyAction::Tap);
        let (label, report) = match action {
            KeyAction::Press => (
                "按键按下",
                self.devices().key_press(&key).await.map_err(to_mcp_error)?,
            ),
            KeyAction::Release => (
                "按键松开",
                self.devices()
                    .key_release(&key)
                    .await
                    .map_err(to_mcp_error)?,
            ),
            KeyAction::Tap => (
                "按键点击",
                self.devices().key_tap(&key).await.map_err(to_mcp_error)?,
            ),
        };
        Ok(text(render(
            label,
            report,
            &[format!("键：{}（{action:?}）", key)],
        )))
    }

    #[tool(
        description = "组合键：列表中最后一个键是主键，其余键在其按下期间保持按住，\
例如 [\"ctrl\", \"shift\", \"s\"] 等价于 Ctrl+Shift+S、[\"alt\", \"f4\"] 等价于 Alt+F4。"
    )]
    async fn keyboard_combo(
        &self,
        Parameters(ComboArgs { keys }): Parameters<ComboArgs>,
    ) -> Result<CallToolResult, McpError> {
        let report = self
            .devices()
            .key_combo(&keys)
            .await
            .map_err(to_mcp_error)?;
        Ok(text(render(
            "组合键",
            report,
            &[format!("组合：{}", keys.join("+"))],
        )))
    }

    #[tool(description = "输入一段文本。仅支持 ASCII 可见字符以及 \\n \\t；\
遇到非 ASCII 字符（例如中文）会直接报错而不会静默跳过。\
需要大写字母与 Shift 符号时服务端会自动按住/释放 Shift。")]
    async fn keyboard_text(
        &self,
        Parameters(TextArgs { text: input }): Parameters<TextArgs>,
    ) -> Result<CallToolResult, McpError> {
        let report = self
            .devices()
            .type_text(&input)
            .await
            .map_err(to_mcp_error)?;
        Ok(text(render(
            "文本输入",
            report,
            &[format!("字符数：{}", input.chars().count())],
        )))
    }

    #[tool(description = "释放本服务此前按下的所有键。被卡住的修饰键（如 Shift）可以用它复位。")]
    async fn keyboard_release_all(&self) -> Result<CallToolResult, McpError> {
        let report = self
            .devices()
            .release_all_keys()
            .await
            .map_err(to_mcp_error)?;
        Ok(text(render("释放所有按键", report, &[])))
    }
}
