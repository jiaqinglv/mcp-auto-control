//! MQTT 发布端。
//!
//! 两个设备固件都**只订阅、不发布**（无 ACK、无 LWT 状态主题），所以这里只做发布：
//! 本层能保证的仅是「报文已交给 broker」，无法保证「设备已执行」。
//!
//! 协议版本使用 MQTT 3.1.1（rumqttc 默认）。固件侧是 MQTT v5，但发布者与订阅者
//! 的协议版本由 broker 各自翻译，互不影响。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use anyhow::{Context, Result};
use rumqttc::{AsyncClient, Event, EventLoop, MqttOptions, Outgoing, Packet, QoS};
use tracing::{debug, info, warn};

use super::codec::{self, KEY_PAYLOAD_LEN, MOUSE_PAYLOAD_LEN};
use crate::config::Config;

/// 客户端发送队列深度。发布是异步的，需要有人持续驱动事件循环。
const CLIENT_QUEUE: usize = 64;

/// 连接断开后的重试间隔（rumqttc 自身也会退避，这里只是兜底）。
const RECONNECT_BACKOFF: Duration = Duration::from_secs(1);

#[derive(Clone)]
pub struct Publisher {
    client: AsyncClient,
    broker: String,
    client_id: String,
    mouse_topic: String,
    keyboard_topic: String,
    frame_interval: Duration,
    next_msg_id: Arc<AtomicU64>,
    connected: Arc<AtomicBool>,
}

impl Publisher {
    /// 创建发布者与事件循环。调用方必须在后台持续 `drive()` 事件循环，
    /// 否则发布队列会填满、所有工具调用都会挂起。
    ///
    /// client id 取 [`Config::resolved_client_id`]：默认带 PID 后缀，避免并行拉起的
    /// 多个实例因 client id 相同而互相踢下线（见该方法的文档）。
    pub fn new(config: &Config) -> (Self, EventLoop) {
        let client_id = config.resolved_client_id();
        let mut options = MqttOptions::new(&client_id, &config.broker, config.port);
        options.set_keep_alive(Duration::from_secs(30));
        options.set_clean_session(true);

        let (client, event_loop) = AsyncClient::new(options, CLIENT_QUEUE);

        let publisher = Self {
            client,
            broker: format!("{}:{}", config.broker, config.port),
            client_id,
            mouse_topic: config.mouse_topic.clone(),
            keyboard_topic: config.keyboard_topic.clone(),
            frame_interval: config.frame_interval(),
            next_msg_id: Arc::new(AtomicU64::new(1)),
            connected: Arc::new(AtomicBool::new(false)),
        };
        (publisher, event_loop)
    }

    pub fn broker(&self) -> &str {
        &self.broker
    }

    /// 本连接实际使用的 MQTT client id（含 PID 后缀）。
    ///
    /// 诊断多实例互相踢下线时，这是区分「是哪个进程在占用会话」的关键信息。
    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    pub fn mouse_topic(&self) -> &str {
        &self.mouse_topic
    }

    pub fn keyboard_topic(&self) -> &str {
        &self.keyboard_topic
    }

    /// 最近一次与 broker 的连接状态。**不代表设备在线**（固件不发任何状态）。
    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }

    /// 后台任务：驱动 MQTT 事件循环并维护连接状态。
    pub async fn drive(&self, mut event_loop: EventLoop) {
        loop {
            match event_loop.poll().await {
                Ok(Event::Incoming(Packet::ConnAck(_))) => {
                    self.connected.store(true, Ordering::Relaxed);
                    info!(broker = %self.broker, client_id = %self.client_id, "已连接 MQTT broker");
                }
                Ok(Event::Incoming(Packet::Disconnect)) => {
                    self.connected.store(false, Ordering::Relaxed);
                    warn!(client_id = %self.client_id, "broker 主动断开连接");
                }
                Ok(Event::Outgoing(Outgoing::Disconnect)) => {
                    self.connected.store(false, Ordering::Relaxed);
                }
                Ok(event) => debug!(?event, "MQTT 事件"),
                Err(err) => {
                    self.connected.store(false, Ordering::Relaxed);
                    warn!(%err, broker = %self.broker, client_id = %self.client_id,
                          "MQTT 连接异常，稍后重试");
                    tokio::time::sleep(RECONNECT_BACKOFF).await;
                }
            }
        }
    }

    /// 下发一帧鼠标报告，返回该帧使用的 `msg_id`（可用于与设备串口日志对账）。
    ///
    /// 每帧后按 `frame_interval` 休眠：设备侧 `MOUSE_CHANNEL` 容量只有 8，
    /// 且接收循环会在通道满时被阻塞。
    pub async fn publish_mouse_frame(
        &self,
        buttons: u8,
        x: i8,
        y: i8,
        wheel: i8,
        pan: i8,
    ) -> Result<u64> {
        let msg_id = self.next_msg_id();
        let payload = codec::encode_mouse(msg_id, buttons, x, y, wheel, pan);
        debug_assert_eq!(payload.len(), MOUSE_PAYLOAD_LEN);
        self.publish(&self.mouse_topic, payload.to_vec(), "鼠标")
            .await?;
        tokio::time::sleep(self.frame_interval).await;
        Ok(msg_id)
    }

    /// 下发一帧按键报告，返回该帧使用的 `msg_id`。
    pub async fn publish_key_frame(&self, key_code: u8, pressed: bool) -> Result<u64> {
        let msg_id = self.next_msg_id();
        let payload = codec::encode_key(msg_id, key_code, pressed);
        debug_assert_eq!(payload.len(), KEY_PAYLOAD_LEN);
        self.publish(&self.keyboard_topic, payload.to_vec(), "键盘")
            .await?;
        tokio::time::sleep(self.frame_interval).await;
        Ok(msg_id)
    }

    fn next_msg_id(&self) -> u64 {
        self.next_msg_id.fetch_add(1, Ordering::Relaxed)
    }

    async fn publish(&self, topic: &str, payload: Vec<u8>, device: &str) -> Result<()> {
        self.client
            .publish(topic, QoS::AtMostOnce, false, payload)
            .await
            .with_context(|| format!("下发{device}报文失败（broker {}）", self.broker))
    }
}
