//! 设备动作语义层。
//!
//! MCP 客户端给出的是高层意图（「移动到 (800,600)」「输入 Hello」「Ctrl+S」），
//! 设备只接受低层二进制帧。这一层负责翻译，并且是**唯一**允许接触
//! [`crate::mqtt::Publisher`] 的地方 —— 因为已经按下的键与按钮必须被集中跟踪，
//! 否则无法实现 `keyboard_release_all` / `emergency_stop`。

pub mod keyboard;
pub mod mouse;

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use tokio::sync::{Mutex, OwnedMutexGuard};

use crate::config::Config;
use crate::mqtt::Publisher;

/// 服务端认为的设备状态。
///
/// 这只是**估计**：固件不回报任何状态（见 `doc/方案设计.md` R1），且物理按键
/// 会产生服务端看不见的位移与按键事件（同文档 R12）。
#[derive(Debug, Clone, Default)]
pub struct DeviceState {
    /// 虚拟光标位置。设备只支持相对位移，绝对坐标由本字段累加而来。
    pub cursor: (i32, i32),
    /// 服务端认为当前被按下的鼠标按钮位掩码。
    pub buttons: u8,
    /// 服务端认为当前被按下的 HID KeyCode 集合。
    pub keys: BTreeSet<u8>,
}

/// 一次动作序列的执行结果，用于回报给 MCP 客户端并与设备串口日志对账。
#[derive(Debug, Clone, Copy)]
pub struct Report {
    pub frames: usize,
    pub first_msg_id: u64,
    pub last_msg_id: u64,
    pub elapsed: Duration,
}

#[derive(Clone)]
pub struct Devices {
    publisher: Publisher,
    config: Arc<Config>,
    state: Arc<Mutex<DeviceState>>,
    /// 保证同一时刻只有一次动作序列在下发，避免鼠标与键盘帧交错，
    /// 同时让帧间隔真正起到限速作用（设备侧通道容量只有 8）。
    ///
    /// 注意：这把锁只保证**动作内部**不被拆散，不保证并发工具调用的相对顺序 ——
    /// rmcp 并发派发请求，谁先拿到锁是不确定的（见 `doc/方案设计.md` R14）。
    sequence: Arc<Mutex<()>>,
}

impl Devices {
    pub fn new(config: &Config, publisher: Publisher) -> Result<Self> {
        let origin = config.mouse_origin()?;
        Ok(Self {
            publisher,
            config: Arc::new(config.clone()),
            state: Arc::new(Mutex::new(DeviceState {
                cursor: origin,
                ..DeviceState::default()
            })),
            sequence: Arc::new(Mutex::new(())),
        })
    }

    pub fn publisher(&self) -> &Publisher {
        &self.publisher
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub async fn state(&self) -> DeviceState {
        self.state.lock().await.clone()
    }

    pub async fn cursor(&self) -> (i32, i32) {
        self.state.lock().await.cursor
    }

    /// 开始一次动作序列：校验 broker 连接、取得独占权。
    ///
    /// broker 未连接时**快速失败**而不排队：rumqttc 会把报文缓存到连接恢复后补发，
    /// 那会让过期的动作在几秒后突然生效。
    pub(crate) async fn begin(&self) -> Result<Sequence> {
        if !self.publisher.is_connected() {
            bail!(
                "MQTT 未连接（broker {}），动作不会下发；请确认 broker 可达后重试",
                self.publisher.broker()
            );
        }
        let guard = self.sequence.clone().lock_owned().await;
        Ok(Sequence {
            publisher: self.publisher.clone(),
            state: self.state.clone(),
            _guard: guard,
            started: Instant::now(),
            frames: 0,
            first_msg_id: 0,
            last_msg_id: 0,
        })
    }

    /// 下发前的帧数预算校验，避免一次调用打爆设备侧通道。
    pub(crate) fn check_budget(&self, frames: usize) -> Result<()> {
        if frames > self.config.max_frames {
            bail!(
                "本次动作需要 {frames} 帧，超过单次上限 {}；请拆分为多次调用或调大 --max-frames",
                self.config.max_frames
            );
        }
        Ok(())
    }
}

/// 一次动作序列。所有帧都经过它下发，状态随之更新。
pub struct Sequence {
    publisher: Publisher,
    state: Arc<Mutex<DeviceState>>,
    _guard: OwnedMutexGuard<()>,
    started: Instant,
    frames: usize,
    first_msg_id: u64,
    last_msg_id: u64,
}

impl Sequence {
    /// 下发一帧鼠标报告，并更新虚拟光标与按钮状态。
    pub async fn mouse_frame(
        &mut self,
        buttons: u8,
        x: i8,
        y: i8,
        wheel: i8,
        pan: i8,
    ) -> Result<u64> {
        let msg_id = self
            .publisher
            .publish_mouse_frame(buttons, x, y, wheel, pan)
            .await?;
        {
            let mut state = self.state.lock().await;
            let (cx, cy) = state.cursor;
            state.cursor = (cx.saturating_add(x as i32), cy.saturating_add(y as i32));
            state.buttons = buttons;
        }
        Ok(self.record(msg_id))
    }

    /// 下发一帧按键报告，并更新已按下键集合。
    pub async fn key_frame(&mut self, key_code: u8, pressed: bool) -> Result<u64> {
        let msg_id = self.publisher.publish_key_frame(key_code, pressed).await?;
        {
            let mut state = self.state.lock().await;
            if pressed {
                state.keys.insert(key_code);
            } else {
                state.keys.remove(&key_code);
            }
        }
        Ok(self.record(msg_id))
    }

    /// 当前是否已按住某个键（由服务端自己按下）。
    pub async fn is_key_down(&self, key_code: u8) -> bool {
        self.state.lock().await.keys.contains(&key_code)
    }

    /// 服务端当前认为被按下的鼠标按钮位掩码。
    pub async fn buttons(&self) -> u8 {
        self.state.lock().await.buttons
    }

    /// 当前按下集合的快照（升序）。
    pub async fn keys_down(&self) -> Vec<u8> {
        self.state.lock().await.keys.iter().copied().collect()
    }

    fn record(&mut self, msg_id: u64) -> u64 {
        if self.frames == 0 {
            self.first_msg_id = msg_id;
        }
        self.last_msg_id = msg_id;
        self.frames += 1;
        msg_id
    }

    pub fn finish(self) -> Report {
        Report {
            frames: self.frames,
            first_msg_id: self.first_msg_id,
            last_msg_id: self.last_msg_id,
            elapsed: self.started.elapsed(),
        }
    }
}
