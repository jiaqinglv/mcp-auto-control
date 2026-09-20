use anyhow::Result;
use clap::Parser;
use mcp_auto_control::config::Config;
use mcp_auto_control::device::Devices;
use mcp_auto_control::mcp::AutoControl;
use mcp_auto_control::mqtt::Publisher;
use rmcp::{ServiceExt, transport::stdio};
use tracing_subscriber::EnvFilter;

/// npx @modelcontextprotocol/inspector cargo run
#[tokio::main]
async fn main() -> Result<()> {
    // 日志一律写 stderr：stdout 是 MCP 协议通道，写任何别的内容都会破坏协议。
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    let config = Config::parse();
    config.validate()?;
    tracing::info!(
        broker = %format!("{}:{}", config.broker, config.port),
        mouse_topic = %config.mouse_topic,
        keyboard_topic = %config.keyboard_topic,
        "启动 mcp-auto-control"
    );

    // MQTT 事件循环必须持续被驱动，否则发布队列会填满、工具调用全部挂起。
    let (publisher, event_loop) = Publisher::new(&config);
    tokio::spawn({
        let publisher = publisher.clone();
        async move { publisher.drive(event_loop).await }
    });

    let devices = Devices::new(&config, publisher)?;
    let service = AutoControl::new(devices)
        .serve(stdio())
        .await
        .inspect_err(|e| tracing::error!("MCP 服务启动失败: {e:?}"))?;

    service.waiting().await?;
    Ok(())
}
