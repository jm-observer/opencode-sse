# `example.rs` 代码审查与改进设计文档

## 1. 现有代码审查

### 1.1 代码概述

`example.rs` 是一个演示程序，连接到 `127.0.0.1:4097`，订阅 SSE（Server-Sent Events）事件流并打印事件数据，同时从 stdin 读取用户输入并 POST 发送到服务器。

### 1.2 发现的问题

#### P0 - 逻辑错误

| # | 问题 | 位置 | 说明 |
|---|------|------|------|
| 1 | **SSE 监听阻塞了 stdin 交互** | L28-L37 → L40-L55 | `while let Some(item) = stream.next().await` 是一个无限循环，只有在服务端关闭连接时才会退出。因此后续的 stdin 交互代码**永远不会执行**。 |
| 2 | **POST 地址与客户端地址不一致** | L13 vs L50 | 客户端初始化使用 `127.0.0.1:4097`，但 POST 请求硬编码为 `http://127.0.0.1:4096`，端口不一致。 |

#### P1 - API 使用不当

| # | 问题 | 位置 | 说明 |
|---|------|------|------|
| 3 | **绕过了生成的客户端 API** | L17-L23 | 直接使用 `openc_client.client.get(...)` 手动拼接 URL 和参数，没有使用已生成的 `event_subscribe()` 方法。客户端库已经提供了 `EventSubscribeResponse::Ok(EventStream<Event>)` 类型安全的流式响应。 |
| 4 | **手动解析 SSE 格式** | L31-L35 | 手动用 `starts_with("data:")` 解析 SSE，但项目已依赖 `sseer` 和 `oas3-gen-support` 的 `EventStream`，应使用库提供的结构化解析。 |
| 5 | **POST 请求无结构化体** | L48-L53 | 发送原始文本 body 到根路径，没有使用已生成的 `session_prompt()` 等结构化 API。 |

#### P2 - 代码质量

| # | 问题 | 位置 | 说明 |
|---|------|------|------|
| 6 | **无用注释** | L4 | `// futures imports removed as not needed`，但 L25 又 `use futures::StreamExt`，注释与实际矛盾。 |
| 7 | **函数体内 `use`** | L25 | `use futures::StreamExt` 放在函数体内，不符合 Rust 惯用风格。 |
| 8 | **地址硬编码** | L13, L50 | 服务器地址直接写死在代码中，不可配置。 |
| 9 | **无错误上下文** | 全文 | 全部使用 `?` 直接传播错误，缺少 `.context()` 提供有意义的错误信息。 |
| 10 | **未使用 SSE 库的 `sseer` 依赖** | Cargo.toml | 项目已声明 `sseer` 依赖但示例中未使用，手动解析 SSE。 |

---

## 2. 改进设计方案

### 2.1 架构改进：并发处理 SSE + stdin

**核心问题**：SSE 监听和 stdin 读取需要并发执行。

**方案**：使用 `tokio::select!` 或 `tokio::spawn` 将两个异步任务并行运行。

```
┌──────────────┐
│   main()     │
│              │
│  ┌───────────┤
│  │ SSE Task  │──→ 接收事件 → 打印/处理
│  ├───────────┤
│  │ Stdin Task│──→ 读取输入 → 调用 session_prompt API
│  └───────────┤
│              │
│  任一退出 →  │──→ 通知另一方 graceful shutdown
└──────────────┘
```

### 2.2 改进后的代码设计

```rust
//! Example: 连接 OpenCode 服务器，并发订阅 SSE 事件和交互式命令行输入

use anyhow::{Context, Result};
use futures::StreamExt;
use opencode_api::types::{
    EventSubscribeRequest, EventSubscribeResponse,
    OpencodeClient, SessionCreateRequest, SessionCreateResponse,
    SessionPromptRequest, SessionPromptRequestPath,
    MessageRequestBody, PartKind2, TextPartInput,
};
use tokio::io::{self, AsyncBufReadExt, BufReader};

const DEFAULT_SERVER: &str = "127.0.0.1:4097";

#[tokio::main]
async fn main() -> Result<()> {
    // 1. 从环境变量或默认值获取服务器地址
    let server_addr = std::env::var("OPENCODE_SERVER")
        .unwrap_or_else(|_| DEFAULT_SERVER.to_string());

    let client = OpencodeClient::with_base_url(&server_addr)
        .context("连接服务器失败")?;

    // 2. 并发运行 SSE 监听和用户交互
    tokio::select! {
        result = sse_listener(&client) => {
            result.context("SSE 监听异常退出")?;
        }
        result = interactive_input(&client) => {
            result.context("交互输入异常退出")?;
        }
    }

    Ok(())
}

/// 订阅 SSE 事件流并处理
async fn sse_listener(client: &OpencodeClient) -> Result<()> {
    let req = EventSubscribeRequest::default();
    let resp = client.event_subscribe(req)
        .await
        .context("订阅 SSE 失败")?;

    match resp {
        EventSubscribeResponse::Ok(mut stream) => {
            println!("已连接，正在监听 SSE 事件...");
            while let Some(event) = stream.next().await {
                match event {
                    Ok(ev) => println!("事件: {:?}", ev),
                    Err(e) => eprintln!("事件解析错误: {}", e),
                }
            }
        }
        EventSubscribeResponse::Unknown => {
            anyhow::bail!("服务器返回未知响应");
        }
    }
    Ok(())
}

/// 从 stdin 读取用户输入并发送到服务器
async fn interactive_input(client: &OpencodeClient) -> Result<()> {
    // 先创建会话，获取 session_id
    let create_req = SessionCreateRequest::default();
    let create_resp = client.session_create(create_req)
        .await
        .context("创建会话失败")?;
    let session_id = match create_resp {
        SessionCreateResponse::Ok(session) => session.id,
        _ => anyhow::bail!("创建会话返回异常"),
    };
    println!("会话已创建: {}", session_id);

    let stdin = io::stdin();
    let mut lines = BufReader::new(stdin).lines();

    println!("--- 交互模式：输入消息发送到服务器 (Ctrl+D 退出) ---");

    while let Some(line) = lines.next_line().await? {
        let text = line.trim().to_string();
        if text.is_empty() {
            continue;
        }

        // 使用生成的类型安全 API
        let req = SessionPromptRequest {
            path: SessionPromptRequestPath {
                session_id: session_id.clone(),
            },
            query: Default::default(),
            body: Some(MessageRequestBody {
                parts: vec![PartKind2::Text(TextPartInput {
                    text,
                    r#type: "text".to_string(),
                    ..Default::default()
                })],
                ..Default::default()
            }),
        };

        match client.session_prompt(req).await {
            Ok(resp) => println!("服务器响应: {:?}", resp),
            Err(e) => eprintln!("发送失败: {}", e),
        }
    }
    Ok(())
}
```

### 2.3 改进要点总结

| 改进项 | 原代码 | 改进后 |
|--------|--------|--------|
| 并发模型 | 顺序执行，stdin 不可达 | `tokio::select!` 并发执行 |
| SSE 解析 | 手动 `starts_with("data:")` | 使用 `EventStream<Event>` 类型安全解析 |
| API 调用 | 绕过客户端，手动拼 URL | 使用 `client.event_subscribe()` / `client.session_prompt()` |
| 服务器地址 | 硬编码，且两处不一致 | 常量 + 环境变量覆盖，统一地址 |
| 错误处理 | 裸 `?` 无上下文 | `.context()` 提供有意义错误信息 |
| 代码结构 | 单个 main 函数 | 拆分为 `sse_listener` / `interactive_input` |
| import 风格 | 函数体内 `use` | 顶部统一 import |

### 2.4 进一步扩展建议

1. **日志系统**：引入 `env_logger` 或 `tracing`，替代 `println!` 调试输出，支持日志级别控制。

2. **重连机制**：SSE 连接断开后自动重连，可使用指数退避策略：
   ```
   断开 → 等待 1s → 重试 → 失败 → 等待 2s → 重试 → 失败 → 等待 4s → ...
   ```

3. **优雅退出**：监听 `Ctrl+C` 信号，清理资源后退出：
   ```rust
   tokio::select! {
       _ = tokio::signal::ctrl_c() => { println!("收到退出信号"); }
       // ... 其他任务
   }
   ```

4. **命令行参数**：使用 `clap` 解析命令行参数，支持 `--server`、`--session-id` 等选项。

5. **Session 管理**：在交互前先调用 `session_create` 创建会话，获取 `session_id`，后续 `session_prompt` 使用该 ID，形成完整的会话流程。
