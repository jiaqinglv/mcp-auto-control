use anyhow::{Context, Result, bail};
use clap::Parser;

/// 命令行参数。stdio 场景下不能污染 stdout，因此所有输出仍走 stderr。
#[derive(Debug, Clone, Parser)]
#[command(
    name = "mcp-auto-control",
    version,
    about = "通过 MQTT 驱动 ESP32 自动鼠标/键盘的 MCP 服务端"
)]
pub struct Config {
    /// MQTT broker 的 IPv4 地址（设备固件只支持 IPv4 字面量）
    #[arg(long, env = "MCP_MQTT_BROKER", default_value = "192.168.3.15")]
    pub broker: String,

    /// MQTT broker 端口
    #[arg(long, env = "MCP_MQTT_PORT", default_value_t = 1883)]
    pub port: u16,

    /// 鼠标设备订阅的主题
    #[arg(long, env = "MCP_MOUSE_TOPIC", default_value = "mouse/auto")]
    pub mouse_topic: String,

    /// 键盘设备订阅的主题
    #[arg(long, env = "MCP_KEYBOARD_TOPIC", default_value = "keyboard/auto")]
    pub keyboard_topic: String,

    /// 本服务端 MQTT client id 的前缀（不能与设备固件中的常量重复）
    ///
    /// 实际使用的 id 见 [`Config::resolved_client_id`]。
    #[arg(long, env = "MCP_CLIENT_ID", default_value = "mcp-auto-control")]
    pub client_id: String,

    /// 是否在 client id 后附加本进程 PID
    ///
    /// MQTT 用 client id 标识会话，重复的 id 会让 broker 踢掉旧会话。本服务常被
    /// 同时拉起多个实例（MCP 客户端预热池 + 当前会话），此时两个实例会以秒级频率
    /// 互相顶掉，表现为「连接时好时坏」。默认开启以保证实例间不冲突。
    #[arg(
        long,
        env = "MCP_UNIQUE_ID_SUFFIX",
        default_value_t = true,
        action = clap::ArgAction::Set
    )]
    pub unique_id_suffix: bool,

    /// 帧间隔（毫秒）。设备侧 MOUSE_CHANNEL 容量为 8，发太快会被丢弃
    #[arg(long, env = "MCP_FRAME_INTERVAL_MS", default_value_t = 10)]
    pub frame_interval_ms: u64,

    /// 单帧最大位移步长，必须 <= 127（设备侧 x/y 是 i8）
    #[arg(long, env = "MCP_MAX_STEP", default_value_t = 100)]
    pub max_step: u8,

    /// 单次工具调用允许下发的最大帧数
    #[arg(long, env = "MCP_MAX_FRAMES", default_value_t = 200)]
    pub max_frames: usize,

    /// 虚拟光标起点，形如 "0,0"。设备只支持相对位移，绝对坐标依赖该估值
    #[arg(long, env = "MCP_MOUSE_ORIGIN", default_value = "0,0")]
    pub mouse_origin: String,
}

impl Config {
    pub fn validate(&self) -> Result<()> {
        if self.max_step == 0 {
            bail!("--max-step 必须大于 0");
        }
        if self.max_step > i8::MAX as u8 {
            bail!(
                "--max-step 必须 <= 127（设备侧 x/y 为 i8），当前为 {}",
                self.max_step
            );
        }
        if self.max_frames == 0 {
            bail!("--max-frames 必须大于 0");
        }
        if self.mouse_topic == self.keyboard_topic {
            bail!("鼠标与键盘主题不能相同：{}", self.mouse_topic);
        }
        if self.mouse_topic.starts_with('$') || self.keyboard_topic.starts_with('$') {
            bail!("主题不能以 $ 开头（MQTT 保留前缀）");
        }
        Ok(())
    }

    pub fn mouse_origin(&self) -> Result<(i32, i32)> {
        let (x, y) = self.mouse_origin.split_once(',').with_context(|| {
            format!(
                "--mouse-origin 格式应为 \"x,y\"，当前为 {}",
                self.mouse_origin
            )
        })?;
        Ok((
            x.trim().parse().context("--mouse-origin 的 x 不是整数")?,
            y.trim().parse().context("--mouse-origin 的 y 不是整数")?,
        ))
    }

    pub fn frame_interval(&self) -> std::time::Duration {
        std::time::Duration::from_millis(self.frame_interval_ms)
    }

    /// 实际使用的 MQTT client id。
    ///
    /// 默认在 [`Config::client_id`] 之后附加本进程 PID，使同一份配置下的多个实例
    /// 各自持有独立会话。注意此时**不再需要**为多开手工设置 `MCP_CLIENT_ID`，
    /// 但同一个 PID 重启后会复用同一个 id（这正是期望的：重启即接管自己的旧会话）。
    pub fn resolved_client_id(&self) -> String {
        if self.unique_id_suffix {
            format!("{}-{}", self.client_id, std::process::id())
        } else {
            self.client_id.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Config {
        Config::parse_from(["mcp-auto-control"])
    }

    #[test]
    fn defaults_match_firmware_constants() {
        let cfg = base();
        assert_eq!(cfg.broker, "192.168.3.15");
        assert_eq!(cfg.port, 1883);
        assert_eq!(cfg.mouse_topic, "mouse/auto");
        assert_eq!(cfg.keyboard_topic, "keyboard/auto");
        // 必须与固件里的 MQTT_CLIENT_ID（AutoMouse / AutoKeyboard）区分开
        assert_ne!(cfg.client_id, "AutoMouse");
        assert_ne!(cfg.client_id, "AutoKeyboard");
        assert_ne!(cfg.resolved_client_id(), "AutoMouse");
        assert_ne!(cfg.resolved_client_id(), "AutoKeyboard");
        cfg.validate().unwrap();
    }

    #[test]
    fn resolved_client_id_is_unique_per_process_by_default() {
        let cfg = base();
        assert!(cfg.unique_id_suffix, "默认必须附加 PID 后缀");
        assert_eq!(
            cfg.resolved_client_id(),
            format!("mcp-auto-control-{}", std::process::id())
        );
    }

    #[test]
    fn resolved_client_id_keeps_the_value_when_suffix_is_disabled() {
        let cfg = Config {
            unique_id_suffix: false,
            client_id: "my-fixed-id".to_string(),
            ..base()
        };
        assert_eq!(cfg.resolved_client_id(), "my-fixed-id");
    }

    #[test]
    fn unique_id_suffix_is_settable_from_cli() {
        // 必须是可关的布尔开关：--unique-id-suffix false 应能关闭
        let cfg = Config::parse_from(["mcp-auto-control", "--unique-id-suffix", "false"]);
        assert!(!cfg.unique_id_suffix);
    }

    #[test]
    fn max_step_above_i8_is_rejected() {
        let cfg = Config {
            max_step: 128,
            ..base()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn identical_topics_are_rejected() {
        let cfg = Config {
            keyboard_topic: "mouse/auto".to_string(),
            ..base()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn origin_parsing() {
        let cfg = Config {
            mouse_origin: "10, -20".to_string(),
            ..base()
        };
        assert_eq!(cfg.mouse_origin().unwrap(), (10, -20));
    }

    #[test]
    fn broken_origin_is_rejected() {
        let cfg = Config {
            mouse_origin: "10".to_string(),
            ..base()
        };
        assert!(cfg.mouse_origin().is_err());
    }
}
