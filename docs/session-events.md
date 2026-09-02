# VibeKeys 多会话事件协议(session events)

> 对应版本:vibekeys_app 0.3.0 起
> 真相来源:`vibekeys_app/src/main.rs`(`SessionEvent` / `claude_event` / `codex_event`)
> 接收方:vibekeys_firmware(设备端维护会话表并负责渲染)

## 背景

键盘要能同时显示多个 agent 会话(Claude Code、Codex 并行)。分工:

- **客户端(vibekeys_app)**:从 hooks 提取结构化信息,打包成单个 JSON 事件发给设备
- **设备端(firmware)**:解析事件、维护会话表(以 `sid` 为 key)、渲染;按最后活跃时间超时移除

## 发送通道

写入现有 **KEYBOARD_DISPLAY** 特性 `cdaa6472-67a8-4241-93cf-145051608573`,单次 writeValue 的 payload:

```json
{"type":"session","ver":1,"sid":"abcd1234","proj":"vibekeys_app","st":"tool"}
```

## 字段说明

| 字段 | 类型 | 必有 | 说明 |
|---|---|---|---|
| `type` | string | 是 | 固定 `"session"`。设备端以此与普通纯文本消息区分 |
| `ver` | u8 | 是 | 协议版本,当前 `1` |
| `sid` | string | 是 | session-id 前 8 位短码(UUID 去连字符也行,前缀含义不变);设备端以此为 key upsert 会话条目 |
| `proj` | string | 是 | workspace 路径最后一段(项目名) |
| `st` | string | 是 | 状态,见下表 |

> 事件只有 `type/ver/sid/proj/st` 五个字段,协议不携带自由文本;设备端只依据 `sid`/`proj`/`st` 渲染,无需处理转义与截断。

### 状态(`st`)枚举

| st | 含义 | Claude Code 来源 | Codex 来源 |
|---|---|---|---|
| `work` | 正在处理用户输入 | UserPromptSubmit、SessionStart | 同左 |
| `tool` | 即将执行工具 | PreToolUse | 同左 |
| `post` | 工具执行完成 | PostToolUse | PostToolUse、SubagentStop |
| `perm` | 等待授权/需注意 | Notification(`permission_prompt`) | PermissionRequest |
| `note` | 空闲等待用户输入 | Notification(`idle_prompt`) | — |
| `done` | 本轮回答结束 | Stop | 同左 |
| `err` | 失败 | StopFailure | — |
| `end` | 会话结束(当前 hooks **不自动发送**,仅供手工 `session` 命令或未来扩展) | — | — |

### 结束信号语义

- `Stop` 只代表一个 turn 结束 → 发 `done`,条目保留
- hooks **不订阅 SessionEnd**:该事件在进程被杀/异常退出时不会发,作为唯一移除信号不可靠
- 会话移除由**设备端超时**负责:每个 `sid` 记录最后活跃时间,超过阈值(固件自定)自动移除条目
- `end` 状态仍保留在协议里,可用于手工清理;相同 `sid` 的新事件按 upsert 处理

## 提取规则(客户端实现)

- `sid`:payload 里完整 UUID 取前 8 个字符(`session_short_id()`)
- `proj`:hook payload 的 `cwd` 最后一段,容忍尾部斜杠(`workspace_name()`)
- 非 JSON 输入、或未列出的事件类型:客户端静默忽略,**不发送**

## 设备端对接要点

1. 收到 DISPLAY 写入先尝试按 JSON 解析;成功且 `"type"=="session"` → 走会话表,否则当纯文本上屏
2. 解析失败也要优雅降级为纯文本(兼容旧版客户端/明文消息)
3. 单次写入远在 MTU 内(整包约 70 字节,正常协商 MTU 247 下非常安全)

## 客户端命令对照

```bash
# hook 链路(stdin 喂 hook JSON,自动提取 sid/proj/st)
echo '{"hook_event_name":"PreToolUse",...}' | vibekeys hook     # Claude Code
echo '{"hook_event_name":"PreToolUse",...}' | vibekeys codex    # Codex

# 手工发一条(调试;只含状态,不含文本)
vibekeys session abcd1234 tool
```

HTTP 层也可直接调 server:

```bash
curl -X POST http://127.0.0.1:42837/send-json -H 'Content-Type: application/json' \
  -d '{"type":"session","ver":1,"sid":"abcd1234","proj":"demo","st":"work"}'
# 缺字段/类型不对返回 400
```
