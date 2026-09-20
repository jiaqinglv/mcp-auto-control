//! 鼠标相关工具。

use rmcp::ErrorData as McpError;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use rmcp::{tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;

use super::super::{AutoControl, render, text, to_mcp_error};
use crate::device::mouse::{MouseButton, ScrollAxis};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MoveArgs {
    /// 水平位移（像素，正数向右，负数向左）
    pub dx: i32,
    /// 垂直位移（像素，正数向下，负数向上）
    pub dy: i32,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MoveToArgs {
    /// 目标 X（虚拟坐标，估计值）
    pub x: i32,
    /// 目标 Y（虚拟坐标，估计值）
    pub y: i32,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MoveSmoothArgs {
    /// 水平位移（像素）
    pub dx: i32,
    /// 垂直位移（像素）
    pub dy: i32,
    /// 拆成多少帧下发；越大轨迹越细，总耗时 = steps × 帧间隔
    pub steps: u32,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ClickArgs {
    /// 按键
    pub button: MouseButton,
    /// 点击次数，默认 1
    pub count: Option<u8>,
    /// 连点间隔（毫秒），默认 0
    pub interval_ms: Option<u64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ButtonArgs {
    /// 按键
    pub button: MouseButton,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ScrollArgs {
    /// 滚动量，正数向上（垂直轴）或向右（水平轴），负数反之
    pub delta: i32,
    /// 滚动轴，默认垂直
    pub axis: Option<ScrollAxis>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DragArgs {
    /// 拖拽的水平位移（像素）
    pub dx: i32,
    /// 拖拽的垂直位移（像素）
    pub dy: i32,
    /// 拖拽时按住的键，默认左键
    pub button: Option<MouseButton>,
}

#[tool_router(router = mouse_tools, vis = "pub(crate)")]
impl AutoControl {
    #[tool(
        description = "相对移动鼠标。dx 正数向右、dy 正数向下；设备只有相对位移能力，\
位移会自动按单帧上限拆成多帧下发。"
    )]
    async fn mouse_move(
        &self,
        Parameters(MoveArgs { dx, dy }): Parameters<MoveArgs>,
    ) -> Result<CallToolResult, McpError> {
        let report = self
            .devices()
            .mouse_move(dx, dy)
            .await
            .map_err(to_mcp_error)?;
        let cursor = self.devices().cursor().await;
        Ok(text(render(
            "鼠标相对移动",
            report,
            &[format!("虚拟光标估计位置：({}, {})", cursor.0, cursor.1)],
        )))
    }

    #[tool(
        description = "把鼠标移动到绝对坐标。设备没有绝对定位能力，这里用服务端维护的虚拟光标\
求增量，因此坐标只是估计值：会因指针加速、屏幕边界与设备物理按键而漂移。"
    )]
    async fn mouse_move_to(
        &self,
        Parameters(MoveToArgs { x, y }): Parameters<MoveToArgs>,
    ) -> Result<CallToolResult, McpError> {
        let before = self.devices().cursor().await;
        let report = self
            .devices()
            .mouse_move_to(x, y)
            .await
            .map_err(to_mcp_error)?;
        let after = self.devices().cursor().await;
        Ok(text(render(
            "鼠标绝对移动",
            report,
            &[format!(
                "虚拟光标：({}, {}) → ({}, {})，目标 ({x}, {y})",
                before.0, before.1, after.0, after.1
            )],
        )))
    }

    #[tool(
        description = "平滑移动鼠标：把位移拆成指定帧数下发，用于需要更细轨迹的场景。\
steps 太小会被拒绝（单帧位移不能超过 --max-step）。"
    )]
    async fn mouse_move_smooth(
        &self,
        Parameters(MoveSmoothArgs { dx, dy, steps }): Parameters<MoveSmoothArgs>,
    ) -> Result<CallToolResult, McpError> {
        let report = self
            .devices()
            .mouse_move_smooth(dx, dy, steps as usize)
            .await
            .map_err(to_mcp_error)?;
        Ok(text(render(
            "鼠标平滑移动",
            report,
            &[format!("共 {steps} 帧")],
        )))
    }

    #[tool(description = "点击鼠标按键，支持连点（同一按键按下后松开，可指定次数与间隔）。")]
    async fn mouse_click(
        &self,
        Parameters(ClickArgs {
            button,
            count,
            interval_ms,
        }): Parameters<ClickArgs>,
    ) -> Result<CallToolResult, McpError> {
        let count = count.unwrap_or(1);
        let interval_ms = interval_ms.unwrap_or(0);
        let report = self
            .devices()
            .mouse_click(button, count, interval_ms)
            .await
            .map_err(to_mcp_error)?;
        Ok(text(render(
            "鼠标点击",
            report,
            &[format!(
                "{} × {count}，间隔 {interval_ms} ms",
                button.label()
            )],
        )))
    }

    #[tool(
        description = "按下鼠标按键并保持（不松开）。松开请调用 mouse_release 或 emergency_stop。"
    )]
    async fn mouse_press(
        &self,
        Parameters(ButtonArgs { button }): Parameters<ButtonArgs>,
    ) -> Result<CallToolResult, McpError> {
        let report = self
            .devices()
            .mouse_press(button)
            .await
            .map_err(to_mcp_error)?;
        Ok(text(render(
            "鼠标按下",
            report,
            &[format!("{} 保持按下", button.label())],
        )))
    }

    #[tool(description = "松开鼠标按键。")]
    async fn mouse_release(
        &self,
        Parameters(ButtonArgs { button }): Parameters<ButtonArgs>,
    ) -> Result<CallToolResult, McpError> {
        let report = self
            .devices()
            .mouse_release(button)
            .await
            .map_err(to_mcp_error)?;
        Ok(text(render(
            "鼠标松开",
            report,
            &[format!("{} 已松开", button.label())],
        )))
    }

    #[tool(
        description = "滚动鼠标滚轮。axis=vertical 时 delta 正数向上、负数向下；\
axis=horizontal 时正数向右、负数向左。"
    )]
    async fn mouse_scroll(
        &self,
        Parameters(ScrollArgs { delta, axis }): Parameters<ScrollArgs>,
    ) -> Result<CallToolResult, McpError> {
        let axis = axis.unwrap_or(ScrollAxis::Vertical);
        let report = self
            .devices()
            .mouse_scroll(delta, axis)
            .await
            .map_err(to_mcp_error)?;
        Ok(text(render(
            "鼠标滚动",
            report,
            &[format!("{:?} 轴，累计 {delta}", axis)],
        )))
    }

    #[tool(
        description = "按住按键拖拽：按下 → 移动 → 松开。即使移动过程中出错也会先松开按键，\
不会把鼠标留在按住状态。"
    )]
    async fn mouse_drag(
        &self,
        Parameters(DragArgs { dx, dy, button }): Parameters<DragArgs>,
    ) -> Result<CallToolResult, McpError> {
        let button = button.unwrap_or(MouseButton::Left);
        let report = self
            .devices()
            .mouse_drag(dx, dy, button)
            .await
            .map_err(to_mcp_error)?;
        Ok(text(render(
            "鼠标拖拽",
            report,
            &[format!("按住{} 位移 ({dx}, {dy})", button.label())],
        )))
    }

    #[tool(
        description = "查询服务端维护的虚拟光标位置。注意这是估计值，不是从操作系统读取的真实\
光标坐标（设备没有回报通道）。"
    )]
    async fn mouse_position(&self) -> Result<CallToolResult, McpError> {
        let cursor = self.devices().cursor().await;
        Ok(text(format!(
            "虚拟光标估计位置：({}, {})\n\
             说明：该值由本服务按已下发的相对位移累加而来，未考虑指针加速、屏幕边界\
以及设备上的物理按键，可能与真实光标位置存在偏差。",
            cursor.0, cursor.1
        )))
    }
}
