# mcp-auto-control 自动控制

提供自动控制功能，包括鼠标移动、点击、滚轮滑动、 键盘输入等。

具体功能实现基于:
·ESP32自动鼠标: <https://github.com/jiaqinglv/AutoMouse>
·ESP32自动键盘: <https://github.com/jiaqinglv/AutoKeyboard>

## 多实例与 client id

MQTT 以 client id 标识会话，重复的 id 会让 broker 踢掉旧会话。本服务常被同时拉起多个
实例（MCP 客户端预热池 + 当前会话），因此 client id 默认附加进程 PID
（形如 `mcp-auto-control-12345`），各实例持有独立会话，互不顶替。

如需固定 id，用 `--unique-id-suffix false`（或 `MCP_UNIQUE_ID_SUFFIX=false`）关闭后缀，
但此时必须为每个实例显式设置不同的 `--client-id`。

当前连接实际使用的 client id 可在 `device_status` 工具的输出中查看。
