# OpenAPI‑SSE Rust 解析器 设计需求文档

## 1. 概述
本项目旨在提供一个轻量级的 Rust 库，用于 **解析 Server‑Sent Events (SSE)**，其中 `data` 字段为符合 `openapi.json` schema 的 JSON。库只负责 **解析并拼装最终结果**，不涉及任何 I/O（如网络请求）。

## 2. 功能需求
| 编号 | 需求描述 | 说明 |
|------|----------|------|
| 1 | 接收原始 SSE 行字符串（UTF‑8） | 逐行喂入解析器。
| 2 | 检测事件边界（空行） | 空行标识事件结束。
| 3 | 解析 `event:` 与 `data:` 字段（忽略其它） | 仅 `data:` 包含 JSON。 |
| 4 | 累积所有 `data:` 行形成完整 JSON | 处理多行 `data:` 的情况。 |
| 5 | 事件结束后，将累积的 JSON 反序列化为对应的 Rust 结构体 | 使用 `serde`。 |
| 6 | 使用 `Result<T, E>` 返回解析结果，其中 `E` 为自定义错误类型 | 符合用户需求。 |
| 7 | 提供异步 API `async fn parse_line(&mut self, line: &str) -> Option<Result<T, E>>` 以及同步包装用于单元测试 | 兼容异步与同步场景。 |
| 8 | 提供 `reset()` 方法以复用解析器 | 便于在同一进程中解析多条流。
| 9 | 只依赖 `serde`（以及 `serde_derive`、`thiserror`） | 依赖最小化。

## 3. 非功能需求
| 编号 | 需求描述 | 细节 |
|------|----------|------|
| N1 | **零 I/O** | 仅接受字符串，不打开网络或文件。
| N2 | **无外部运行时** | 只依赖标准库和 `serde`。
| N3 | **线程安全** | `Parser` 实现 `Send + Sync`。
| N4 | **完整文档** | 使用 `///` 注释并提供示例代码。
| N5 | **测试覆盖率 ≥ 90%** | 包括正常、错误、边界情况。
| N6 | **友好错误** | 自定义 `ParseError`，包括 `InvalidEvent`、`JsonError` 等。
| N7 | **语义化版本** | 初始版本 `0.1.0`。

## 4. 架构设计
```
crate
├─ src/
│  ├─ lib.rs          ← 公共 API（pub struct Parser, Error, Result）
│  ├─ parser.rs       ← 负责状态机与行解析
│  ├─ model.rs        ← 与 openapi.json 对应的结构体（serde）
│  └─ error.rs        ← 自定义错误类型
└─ tests/
   └─ integration.rs ← 集成测试（模拟完整 SSE 流）
```

### 4.1 数据模型 (`model.rs`)
```rust
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SsePayload {
    // 以下字段请依据 openapi.json 进行补全
    pub id: String,
    pub object: String,
    pub created: i64,
    pub model: String,
    // …其他字段…
}
```
> 结构体字段需与 `openapi.json` 完全匹配。

### 4.2 解析器状态机 (`parser.rs`)
| 状态 | 说明 |
|------|------|
| Idle | 尚未收到任何 `data:` 行 |
| Collecting | 已收到一条或多条 `data:` 行，正在累积 JSON |
| Completed | 收到空行，尝试反序列化 |
| Error | 遇到不可恢复的错误，需要调用 `reset()` 恢复 |

核心实现示例（已在注释中解释）：
```rust
pub struct Parser {
    buffer: String,
    done: bool,
}

impl Parser {
    pub fn new() -> Self { Self { buffer: String::new(), done: false } }

    /// 逐行喂入 SSE 文本，返回 `Some(Ok(payload))` 表示完整事件解析完成，
    /// `Some(Err(e))` 为解析错误，`None` 表示需要更多行。
    pub fn parse_line(&mut self, line: &str) -> Option<Result<SsePayload, ParseError>> {
        match line.trim_end() {
            "" => {
                // 事件结束，尝试反序列化
                self.done = true;
                let json = std::mem::take(&mut self.buffer);
                let result = serde_json::from_str::<SsePayload>(&json)
                    .map_err(ParseError::Json);
                Some(result)
            }
            l if l.starts_with("data:") => {
                // 只保留 data: 之后的内容（包括可能的换行）
                let payload = l["data:".len()..].trim_start();
                self.buffer.push_str(payload);
                None
            }
            _ => None, // 其它字段直接忽略
        }
    }

    /// 重置内部状态，以便再次解析新流。
    pub fn reset(&mut self) {
        self.buffer.clear();
        self.done = false;
    }
}
```

### 4.3 错误类型 (`error.rs`)
```rust
#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("invalid SSE 行")]
    InvalidLine,
    #[error("JSON 反序列化失败")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, ParseError>;
```

## 5. 实现计划 (Plan)
| 步骤 | 目标 | 方式 |
|------|------|------|
| P1 | 建立 Cargo 项目骨架 & 添加依赖 (`serde`, `serde_derive`, `thiserror`) | `cargo new sse_parser --lib`，编辑 `Cargo.toml`。
| P2 | 编写 `model.rs`（用 `openapi.json` 生成的结构体） | 手动粘贴或使用 `schemars` 自动生成。
| P3 | 实现 `Parser` 与状态机逻辑 | 编写 `parser.rs` 并在 `lib.rs` 暴露。
| P4 | 定义 `ParseError` 与 `Result` 类型 | `error.rs`。
| P5 | 编写单元测试覆盖正常、多行、错误、空事件等情形 | `tests/integration.rs`。
| P6 | 编写文档 (`README.md`, `DESIGN.md`) 并在 `doc/` 中放置本文件 | 使用 `cargo doc` 生成 API 文档。
| P7 | 本地运行 `cargo test` 确保全部通过，随后提交代码 |

## 6. 测试策略
| 测试类型 | 场景 | 期望结果 |
|----------|------|----------|
| 单元‑正常 | 单行 `data:` + 空行 | 返回 `Ok(payload)` |
| 单元‑多行 | 两条 `data:` 行后空行 | 两行内容拼接后成功反序列化 |
| 单元‑错误JSON | `data: {invalid}` + 空行 | 返回 `Err(ParseError::Json)` |
| 单元‑意外行 | 包含 `retry:`、`id:` 等非 `data:` 行 | 仍返回 `None` 直至空行 |
| 集成‑完整流 | 使用 `response` 目录下的示例文件模拟完整 SSE 流 | 解析得到预期结构体 |
| 边界‑空事件 | 直接收到空行 | 返回 `None`（无 payload） |
| 边界‑重置 | 成功解析后调用 `reset()` 再次喂入新事件 | 第二次解析成功，状态不受前一次影响 |

## 7. 文档计划
1. **`README.md`** – 快速入门示例代码。
2. **`DESIGN.md`** – 本设计文档（即本文件），放在 `doc/` 目录。
3. **API 文档** – 使用 `cargo doc` 自动生成。
4. **测试说明** – 在 `README.md` 中给出 `cargo test` 的使用方式。

## 8. 未来可选扩展（非必需）
| 功能 | 说明 |
|------|------|
| Tokio 异步流支持 | 提供 `async fn parse_stream<S>(stream: S) where S: Stream<Item = Result<String, E>>`，便于直接与 `reqwest` SSE 响应配合使用。 |
| 事件类型区分 | 解析 `event:` 行并返回 `enum EventKind { Message, Error, ... }` |
| 自定义错误类型 | 让用户自行实现 `enum MyError` 并通过 `Result<T, MyError>` 使用库。 |
| 多种序列化格式 | 支持 `serde_json` 之外的 `serde_cbor`、`serde_yaml`（特性开关）。 |

## 9. 交付清单
- [ ] Cargo 项目骨架 (`Cargo.toml`, `src/` 目录)。
- [ ] 完整实现的 `Parser`、`ParseError`、`Result`。
- [ ] `model.rs`（结构体占位，用户自行填充字段）。
- [ ] 单元测试与集成测试代码。
- [ ] `doc/design.md`（本文件的中文版本）。
- [ ] `README.md` 示例。
- [ ] 本地 `cargo test` 通过，准备提交。

如无其他需求，请确认上述计划，我将开始实现代码并提交到仓库。