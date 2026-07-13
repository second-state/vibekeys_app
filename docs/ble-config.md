# VibeKeys 固件 BLE 配置协议

> 对应固件分支:`feature/mqtt-vibetty-integration`(HEAD `b4083df`)
> 文档对象:vibekeys_app 等需要对接固件 BLE 配置的客户端
> 真相来源:`vibekeys_firmware/src/bt_wifi_mode.rs`、`src/audio.rs`、`assets/setup.html`

## 背景

本次固件分支对 BLE 设备配置做了一次重构:把过去**散落在多个特性、多个写入端点**的配置项,统一收敛到**单一 CONFIG 特性**上,并把写入协议从 `{type, value}` 改成**部分对象(patch)**语义,与读取返回的整份快照对称。

目标:减少客户端往返次数、支持增量更新、消除同一份数据的多条存储路径。

---

## 一、变化总结(旧 → 新)

### 1. 统一到单一 CONFIG 特性 `de1b978`
| | 旧 | 新 |
|---|---|---|
| ASR 配置 | 专用特性 `KEYMAP_ASR_CONFIG_ID` + 命令 `ControllerCommand::AsrConfig` + `handle_keymap_asr_config` | CONFIG 特性的 `asr_config` 字段 |
| mic mode | 专用特性 `MIC_MODEL_ID` | CONFIG 特性的 `mic_model` 字段 |
| server_url | 曾有专用特性 | CONFIG 特性的 `server_url` 字段 |

旧的 `KEYMAP_ASR_CONFIG_ID`、`MIC_MODEL_ID` 特性及对应处理函数**已全部删除**。CONFIG 特性(`cef520a9-…`)现在承载:`wifi_list` / `server_url` / `asr_config` / `mic_model` / `prefer_builtin_asr`。

### 2. 写入线格式:`{type,value}` → 部分对象 `85a7af9`
- **旧**:`{"type":"wifi_list|server_url|asr_config|mic_model","value":...}` —— 单字段、单次发,改多项得发多次。
- **新**:`{wifi_list?, server_url?, asr_config?, mic_model?, prefer_builtin_asr?}` —— 一次写可携带任意多项,设备只更新**出现(非 `null`/非缺省)的字段**,缺失字段保持原状。
- 读取协议**不变**,仍返回整份快照。

### 3. `asr_config` 增量合并(三层) `9fd1331`
写 `asr_config` 不再整包替换,而是逐 key 合并:

```
默认值  <  现有 NVS  <  本次传入
```

例:只发 `{"asr_config":{"api_key":"xxx"}}` 仅更新 `api_key`,`uri` / `model` 原样保留。合并后的对象仍按 `AsrConfig` 校验,非法则不落盘。其余字段类型不受影响。

### 4. 新增 `prefer_builtin_asr` + 修复 NVS key 超长 `f4f4fcc` `5912a5c`
- 含义:键盘模式下是否优先用内置 ASR(Whisper);`false` 时 MIC 透传给主机,触发主机自带听写。
- **坑**:NVS key 上限 15 字符,`prefer_builtin_asr`(18)会触发 `ESP_ERR_NVS_KEY_TOO_LONG` → 实际没写进 flash → 重启读回默认 `true`,表现成「写 `false`、读还是 `true`」。
- 修复:实际 NVS key 用 **`prefer_asr`(10 字符)**;JSON 字段名、Rust 字段名仍为 `prefer_builtin_asr`,**客户端无感**。

### 5. `mic_model` 移入 CONFIG + falsy 保留 `de1b978`
- mic mode 不再有专用特性,并入 CONFIG 快照读写。
- `writeConfig` 不再把 falsy 值强转,确保 `mic_model = 0`(PTT)能存活(随后被 `writeConfigPatch` 取代)。

### 6. 配套行为
- `prefer_builtin_asr` 默认 `true`;`mic_model` 默认 `1`(Toggle)。
- `wifi_list` 上限 **8 条**(NVS 单值 ~4KB 限额,超出截断)。
- 保存配置后,客户端应向 `RESET` 特性发 `RESET` 触发重启,使配置生效(`asr_config` 等在 boot 阶段重新加载)。

---

## 二、当前协议规范

### GATT 服务与特性
| 特性 | UUID | 属性 | 用途 |
|---|---|---|---|
| Service | `623fa3e2-631b-4f8f-a6e7-a7b09c03e7e0` | — | 主服务 |
| **CONFIG** | `cef520a9-bcb5-4fc6-87f7-82804eee2b20` | READ \| WRITE | 统一配置读写 |
| BACKGROUND_PNG | `d1f3b2c4-5e6f-4a7b-8c9d-0e1f2a3b4c5d` | WRITE | 分块上传背景图 |
| RESET | `f0e1d2c3-b4a5-6789-0abc-def123456789` | WRITE | 写入 `RESET` 触发重启 |

### CONFIG 读取 → 整份快照
一次 `readValue` 返回:

```json
{
  "wifi_list": [{ "ssid": "Foo", "pass": "bar" }],
  "server_url": "https://asr.example.com",
  "asr_config": { "platform": "whisper", "uri": "...", "api_key": "...", "model": "..." },
  "mic_model": 1,
  "prefer_builtin_asr": true
}
```

字段说明:
- `wifi_list`:数组,顺序即连接优先级;可能为空 `[]`。
- `asr_config`:未配置时该字段**省略**(`skip_serializing_if = Option::is_none`)。当前仅支持 `platform: "whisper"`(`uri` / `api_key` / `model`)。
- `mic_model`:`0` = PTT(按住说话),`1` = Toggle(点按切换)。旧固件无此字段时,客户端应默认 `1`。
- `prefer_builtin_asr`:旧固件无此字段时,客户端应默认 `true`。

### CONFIG 写入 → 部分对象
发送一个 JSON 对象,**只包含要更新的字段**:

```json
{ "server_url": "https://new.url", "prefer_builtin_asr": false }
```

| 字段 | 类型 | 语义 |
|---|---|---|
| `wifi_list` | `[{ssid,pass}]` | 整包替换,最多 8 条,超出截断;顺序即优先级 |
| `server_url` | string | 整包替换 |
| `asr_config` | object | **三层合并**(默认 < 现有 NVS < 传入),只覆盖传入中出现的 key |
| `mic_model` | `0` \| `1` | 0=PTT,1=Toggle |
| `prefer_builtin_asr` | bool | 键盘模式是否优先内置 ASR |

> 空对象 `{}` 或缺省字段 = no-op,不落盘。改多项**只发一次** writeValue 即可。

### 客户端推荐流程(参考 setup.html)
1. 连接设备 → `getCharacteristic(CONFIG_ID)`。
2. **Load**:`readValue()` 一次拿快照 → 填充 UI。
3. **Save**:收集改动字段攒成 patch(无改动时可发整份)→ `writeValue(CONFIG, patch)` 一次 → `writeValue(RESET, "RESET")` 重启生效。
4. 提示「Saved N field(s) in 1 write」。

---

## 三、NVS 存储布局

| NVS key | 类型 | 对应 JSON 字段 | 备注 |
|---|---|---|---|
| `wifi_list` | str(JSON) | `wifi_list` | 单个 JSON 值,≤ ~4KB,最多 8 条 |
| `server_url` | str | `server_url` | |
| `asr_config` | str(JSON) | `asr_config` | `AsrConfig`(`#[serde(tag="platform")]`) |
| `mic_model` | u8 | `mic_model` | 0 / 1 |
| `prefer_asr` | u8 | `prefer_builtin_asr` | **key 缩写**,值 0 / 1;JSON 字段名仍是长名 |
| `background_png` | blob | — | 背景图 |
| `state` | u8 | — | 初始化标志(读出 1 或 wifi/url 空 → 进入配网) |

⚠️ **NVS key 长度上限 15 字符**。`prefer_asr` 即为此而生;新增 key 务必自行检查长度。

---

## 四、兼容性 / 迁移注意

- 旧的 `ssid` / `pass` 分散 NVS 数据**不迁移**;`wifi_list` 一律以单个 JSON 值存放。
- CONFIG 特性为本分支新增,目前仅 `setup.html` 使用。**移除旧端点意味着**走 `KEYMAP_ASR_CONFIG_ID` / `MIC_MODEL_ID` 的旧客户端必须升级,否则配置写入会失败。
- 读取协议未变,只有写入协议从 `{type,value}` 变为部分对象。

---

## 五、参考实现指针

| 内容 | 位置 |
|---|---|
| CONFIG 特性创建 / `on_read` / `on_write` | `vibekeys_firmware/src/bt_wifi_mode.rs:184-302` |
| `ConfigSnapshot`(读)/ `ConfigSaveSnapshot`(写)结构 | `bt_wifi_mode.rs:28-45` |
| `asr_config` 三层合并逻辑 | `bt_wifi_mode.rs:265-287` |
| `AsrConfig` enum + `save_to_nvs` / `load_from_nvs` | `vibekeys_firmware/src/audio.rs:301-342` |
| 客户端读写实现 | `vibekeys_firmware/assets/setup.html`(`loadAllConfiguration` / `saveAllModifications` / `writeConfigPatch`) |
