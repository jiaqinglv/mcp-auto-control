//! 工具注册的集成测试。
//!
//! 工具定义分散在 `src/mcp/tools/` 的三个 `#[tool_router]` 块里，靠
//! `ToolRouter` 的 `+` 合并。这个测试保证合并没有漏掉任何一域 —— 也正是
//! 引入 lib target 的主要动机（binary-only crate 无法被 `tests/` 引用）。

use std::collections::BTreeSet;

use mcp_auto_control::mcp::AutoControl;

/// 期望注册的工具全集。增删工具时同步更新这里。
const EXPECTED_TOOLS: &[&str] = &[
    // 鼠标
    "mouse_move",
    "mouse_move_to",
    "mouse_move_smooth",
    "mouse_click",
    "mouse_press",
    "mouse_release",
    "mouse_scroll",
    "mouse_drag",
    "mouse_position",
    // 键盘
    "keyboard_key",
    "keyboard_combo",
    "keyboard_text",
    "keyboard_release_all",
    // 系统
    "device_status",
    "emergency_stop",
];

fn registered_names() -> BTreeSet<String> {
    AutoControl::all_tools()
        .list_all()
        .into_iter()
        .map(|tool| tool.name.to_string())
        .collect()
}

#[test]
fn merged_router_registers_every_tool() {
    let registered = registered_names();
    let expected: BTreeSet<String> = EXPECTED_TOOLS.iter().map(|name| name.to_string()).collect();

    let missing: Vec<_> = expected.difference(&registered).collect();
    let unexpected: Vec<_> = registered.difference(&expected).collect();
    assert!(
        missing.is_empty() && unexpected.is_empty(),
        "工具集合与预期不一致：缺少 {missing:?}，多出 {unexpected:?}"
    );
}

#[test]
fn every_tool_documents_itself() {
    for tool in AutoControl::all_tools().list_all() {
        let description = tool
            .description
            .as_ref()
            .map(|d| d.to_string())
            .unwrap_or_default();
        assert!(
            !description.trim().is_empty(),
            "工具 {} 缺少 description：MCP 客户端只能靠它判断用途",
            tool.name
        );
    }
}

#[test]
fn every_tool_has_a_json_schema() {
    for tool in AutoControl::all_tools().list_all() {
        // rmcp 由 schemars 生成 inputSchema；无参工具也应是对象类型
        assert_eq!(
            tool.input_schema.get("type").and_then(|v| v.as_str()),
            Some("object"),
            "工具 {} 的 inputSchema 不是 object",
            tool.name
        );
    }
}
