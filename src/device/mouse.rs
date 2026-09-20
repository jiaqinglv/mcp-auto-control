//! 鼠标动作语义：虚拟光标、位移拆分、点击/滚轮/拖拽。
//!
//! 设备侧 `x` / `y` 是 **i8 相对增量**（`AutoMouse/src/mqtt.rs:151-155`），
//! 所以「移动 1000 像素」必须拆成多帧，且每帧不能超过 i8 范围。

use anyhow::{Result, bail};
use schemars::JsonSchema;
use serde::Deserialize;

use super::{Devices, Report};
use crate::mqtt::{BUTTON_LEFT, BUTTON_MIDDLE, BUTTON_RIGHT};

/// 单次调用允许请求的最大点击次数，防止误触发连点。
const MAX_CLICK_COUNT: u8 = 10;
/// 连点间隔上限（毫秒）。
const MAX_CLICK_INTERVAL_MS: u64 = 5_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum MouseButton {
    /// 左键（位掩码 0x01）
    Left,
    /// 右键（位掩码 0x02）
    Right,
    /// 中键（位掩码 0x04）
    Middle,
}

impl MouseButton {
    pub fn mask(self) -> u8 {
        match self {
            MouseButton::Left => BUTTON_LEFT,
            MouseButton::Right => BUTTON_RIGHT,
            MouseButton::Middle => BUTTON_MIDDLE,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            MouseButton::Left => "左键",
            MouseButton::Right => "右键",
            MouseButton::Middle => "中键",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ScrollAxis {
    /// 垂直滚动：正数向上
    Vertical,
    /// 水平滚动：正数向右
    Horizontal,
}

/// 把 `(dx, dy)` 均匀拆成恰好 `steps` 帧，保证各帧累加后精确等于输入。
///
/// 余数补偿在靠后的帧上（整除 + 递推），因此不会出现「差几个像素」的累计误差。
pub fn split_delta(dx: i32, dy: i32, steps: usize) -> Result<Vec<(i8, i8)>> {
    if steps == 0 {
        bail!("steps 必须大于 0");
    }
    if dx == 0 && dy == 0 {
        return Ok(Vec::new());
    }

    let mut frames = Vec::with_capacity(steps);
    let mut rem_x = dx;
    let mut rem_y = dy;

    for remaining_frames in (1..=steps).rev() {
        let step_x = rem_x / remaining_frames as i32;
        let step_y = rem_y / remaining_frames as i32;
        rem_x -= step_x;
        rem_y -= step_y;
        let frame = (
            i8::try_from(step_x).map_err(|_| {
                anyhow::anyhow!("单帧 x 位移 {step_x} 超出 i8 范围，请增大 steps 或减小位移")
            })?,
            i8::try_from(step_y).map_err(|_| {
                anyhow::anyhow!("单帧 y 位移 {step_y} 超出 i8 范围，请增大 steps 或减小位移")
            })?,
        );
        frames.push(frame);
    }

    debug_assert_eq!(
        frames.iter().map(|(x, _)| *x as i32).sum::<i32>(),
        dx,
        "拆分后 x 必须精确加和"
    );
    debug_assert_eq!(
        frames.iter().map(|(_, y)| *y as i32).sum::<i32>(),
        dy,
        "拆分后 y 必须精确加和"
    );
    Ok(frames)
}

/// 按 `max_step` 计算需要多少帧才能走完 `(dx, dy)`。
pub fn frames_needed(dx: i32, dy: i32, max_step: u8) -> usize {
    let max_step = max_step.max(1) as i32;
    let max_abs = dx.abs().max(dy.abs());
    if max_abs == 0 {
        return 0;
    }
    ((max_abs + max_step - 1) / max_step) as usize
}

impl Devices {
    /// 相对移动。自动按 `max_step` 拆分帧。
    pub async fn mouse_move(&self, dx: i32, dy: i32) -> Result<Report> {
        let steps = frames_needed(dx, dy, self.config().max_step);
        self.mouse_move_in_steps(dx, dy, steps.max(1)).await
    }

    /// 绝对移动。设备没有绝对定位能力，这里用虚拟光标求增量。
    ///
    /// 结果受指针加速与屏幕边界影响，返回值中的坐标是**估计值**。
    pub async fn mouse_move_to(&self, x: i32, y: i32) -> Result<Report> {
        let (cx, cy) = self.cursor().await;
        self.mouse_move(x.saturating_sub(cx), y.saturating_sub(cy))
            .await
    }

    /// 指定帧数的平滑移动，用于需要更细轨迹的场景。
    pub async fn mouse_move_smooth(&self, dx: i32, dy: i32, steps: usize) -> Result<Report> {
        let needed = frames_needed(dx, dy, self.config().max_step);
        if steps < needed.max(1) {
            bail!(
                "steps={steps} 太小：位移 ({dx}, {dy}) 至少需要 {needed} 帧（单帧上限 {}）",
                self.config().max_step
            );
        }
        self.mouse_move_in_steps(dx, dy, steps).await
    }

    async fn mouse_move_in_steps(&self, dx: i32, dy: i32, steps: usize) -> Result<Report> {
        let frames = split_delta(dx, dy, steps)?;
        self.check_budget(frames.len())?;

        let mut sequence = self.begin().await?;
        let buttons = sequence.buttons().await;
        for (x, y) in frames {
            sequence.mouse_frame(buttons, x, y, 0, 0).await?;
        }
        Ok(sequence.finish())
    }

    /// 点击（按下 + 松开），支持连点。
    pub async fn mouse_click(
        &self,
        button: MouseButton,
        count: u8,
        interval_ms: u64,
    ) -> Result<Report> {
        if count == 0 {
            bail!("count 必须大于 0");
        }
        if count > MAX_CLICK_COUNT {
            bail!("count={count} 超过上限 {MAX_CLICK_COUNT}");
        }
        if interval_ms > MAX_CLICK_INTERVAL_MS {
            bail!("interval_ms={interval_ms} 超过上限 {MAX_CLICK_INTERVAL_MS}");
        }
        let frames = count as usize * 2;
        self.check_budget(frames)?;

        let mut sequence = self.begin().await?;
        let mask = button.mask();
        let current = sequence.buttons().await;
        for _ in 0..count {
            sequence.mouse_frame(current | mask, 0, 0, 0, 0).await?;
            sequence.mouse_frame(current, 0, 0, 0, 0).await?;
            if interval_ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(interval_ms)).await;
            }
        }
        Ok(sequence.finish())
    }

    /// 按下鼠标按钮并保持。
    pub async fn mouse_press(&self, button: MouseButton) -> Result<Report> {
        self.check_budget(1)?;
        let mut sequence = self.begin().await?;
        let buttons = sequence.buttons().await | button.mask();
        sequence.mouse_frame(buttons, 0, 0, 0, 0).await?;
        Ok(sequence.finish())
    }

    /// 松开鼠标按钮。
    pub async fn mouse_release(&self, button: MouseButton) -> Result<Report> {
        self.check_budget(1)?;
        let mut sequence = self.begin().await?;
        let buttons = sequence.buttons().await & !button.mask();
        sequence.mouse_frame(buttons, 0, 0, 0, 0).await?;
        Ok(sequence.finish())
    }

    /// 滚轮滚动。正数向上 / 向右，负数反之。
    pub async fn mouse_scroll(&self, delta: i32, axis: ScrollAxis) -> Result<Report> {
        if delta == 0 {
            bail!("delta 不能为 0");
        }
        let steps = frames_needed(delta, 0, self.config().max_step);
        let frames = split_delta(delta, 0, steps.max(1))?;
        self.check_budget(frames.len())?;

        let mut sequence = self.begin().await?;
        let buttons = sequence.buttons().await;
        for (amount, _) in frames {
            let (wheel, pan) = match axis {
                ScrollAxis::Vertical => (amount, 0),
                ScrollAxis::Horizontal => (0, amount),
            };
            sequence.mouse_frame(buttons, 0, 0, wheel, pan).await?;
        }
        Ok(sequence.finish())
    }

    /// 拖拽：按住按钮 → 移动 → 松开。松开始终会执行，即使移动中途失败。
    pub async fn mouse_drag(&self, dx: i32, dy: i32, button: MouseButton) -> Result<Report> {
        let steps = frames_needed(dx, dy, self.config().max_step).max(1);
        let frames = split_delta(dx, dy, steps)?;
        self.check_budget(frames.len() + 2)?;

        let mut sequence = self.begin().await?;
        let mask = button.mask();
        let base = sequence.buttons().await;
        sequence.mouse_frame(base | mask, 0, 0, 0, 0).await?;

        let mut move_error = None;
        for (x, y) in frames {
            if let Err(err) = sequence.mouse_frame(base | mask, x, y, 0, 0).await {
                move_error = Some(err);
                break;
            }
        }

        // 无论移动是否成功，都先松开按钮，避免卡在按住状态。
        sequence.mouse_frame(base, 0, 0, 0, 0).await?;
        let report = sequence.finish();
        match move_error {
            Some(err) => Err(err),
            None => Ok(report),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_is_exact_for_negative_and_positive() {
        for (dx, dy) in [(250, 0), (-250, 0), (300, 300), (-1, 1), (7, -3), (1, 1)] {
            let steps = frames_needed(dx, dy, 100);
            let frames = split_delta(dx, dy, steps).unwrap();
            assert_eq!(frames.len(), steps);
            assert_eq!(frames.iter().map(|(x, _)| *x as i32).sum::<i32>(), dx);
            assert_eq!(frames.iter().map(|(_, y)| *y as i32).sum::<i32>(), dy);
        }
    }

    #[test]
    fn split_respects_max_step() {
        let frames = split_delta(1000, -750, frames_needed(1000, -750, 100)).unwrap();
        for (x, y) in frames {
            assert!(x.unsigned_abs() <= 100, "x={x} 超出单帧上限");
            assert!(y.unsigned_abs() <= 100, "y={y} 超出单帧上限");
        }
    }

    #[test]
    fn zero_delta_produces_no_frames() {
        assert!(split_delta(0, 0, 5).unwrap().is_empty());
        assert_eq!(frames_needed(0, 0, 100), 0);
    }

    #[test]
    fn too_few_steps_is_rejected_instead_of_wrapping() {
        // 500 / 2 = 250，超出 i8，必须报错而不是截断成 i8
        let err = split_delta(500, 0, 2).unwrap_err();
        assert!(err.to_string().contains("超出 i8"));
    }

    #[test]
    fn zero_steps_is_rejected() {
        assert!(split_delta(1, 1, 0).is_err());
    }

    #[test]
    fn frames_needed_is_ceiling_division() {
        assert_eq!(frames_needed(100, 0, 100), 1);
        assert_eq!(frames_needed(101, 0, 100), 2);
        assert_eq!(frames_needed(0, 127, 127), 1);
        assert_eq!(frames_needed(0, 128, 127), 2);
    }

    #[test]
    fn button_masks_from_firmware() {
        assert_eq!(MouseButton::Left.mask(), 0x01);
        assert_eq!(MouseButton::Right.mask(), 0x02);
        assert_eq!(MouseButton::Middle.mask(), 0x04);
    }
}
