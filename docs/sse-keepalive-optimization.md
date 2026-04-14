# SSE 连接保活与重连优化方案

## 1. 问题现状

`example.rs` 中 SSE 连接很快断开，核心链路如下：

```
OpencodeClient::event_subscribe()
  → reqwest GET /event
    → EventSubscribeResponse::Ok(EventStream<Event>)
      → oas3-gen-support EventStream (eventsource_stream)
        → while let Some(event) = stream.next().await { ... }
```

整个链路中**没有任何保活/重连机制**。

### 1.1 缺失项一览

| 缺失项 | 位置 | 说明 |
|---|---|---|
| reqwest 无 TCP keepalive | `client.rs:32` | `Client::builder().build()` 使用默认配置 |
| reqwest 无禁用超时 | `client.rs:32` | 默认超时会中断 SSE 长连接 |
| EventStream 无心跳处理 | `oas3-gen-support event_stream.rs` | 空 data 跳过，不主动检测连接存活 |
| stream 结束即退出 | `example.rs:63-96` | `Poll::Ready(None)` 后无重连 |
| 无超时检测 | `example.rs` 全文 | 没有"多久没收到数据就认为断连"的机制 |

## 2. 参考：JetBrains IdeBridge 的保活设计

IdeBridge（`IdeBridge.kt`）作为 SSE **服务端**，实现了完整的保活体系：

### 2.1 心跳机制（每 15 秒）

```kotlin
// IdeBridge.kt ~行 94-103
keepaliveTimer = Timer("IdeBridge-Keepalive", true).apply {
    scheduleAtFixedRate(object : TimerTask() {
        override fun run() { sendKeepaliveToAll() }
    }, 15000, 15000)  // 每 15 秒
}
```

心跳内容为 SSE 注释格式 `: ping\n\n`，**不会触发客户端事件回调**，但能保持 TCP 连接不被中间代理/防火墙超时关闭。

### 2.2 死连接自动清理

```kotlin
// IdeBridge.kt ~行 143-162
private fun sendKeepaliveToAll() {
    session.sseClients.forEach { client ->
        try {
            writer.write(": ping\n\n")
            writer.flush()
        } catch (e: Exception) {
            toRemove.add(client)  // write 失败 → 标记移除
        }
    }
    toRemove.forEach { session.sseClients.remove(it) }
}
```

### 2.3 消息广播中同步清理

```kotlin
// IdeBridge.kt ~行 469-486
private fun broadcastSSE(session: Session, json: String) {
    synchronized(session.sseClients) {
        // 写入失败的 client 也会被移除
    }
}
```

### 2.4 SSE 响应头设置

```kotlin
// IdeBridge.kt ~行 219-249
exchange.responseHeaders.apply {
    add("Content-Type", "text/event-stream")
    add("Cache-Control", "no-cache, no-transform")
    add("Connection", "keep-alive")
    add("X-Accel-Buffering", "no")  // 禁止 nginx 缓冲
}
exchange.sendResponseHeaders(200, 0)  // length=0 表示流式响应
```

### 2.5 对比总结

| 机制 | IdeBridge (服务端) | opencode-sse (客户端) |
|---|---|---|
| 心跳发送 | 每 15 秒 `: ping\n\n` | 无（依赖服务端发） |
| 死连接检测 | write 失败立即移除 | 无 |
| TCP keepalive | HTTP Server 层面保持 | reqwest 未配置 |
| 断线清理 | `sendKeepaliveToAll()` 自动清理 | 无 |
| 连接复用 | `Connection: keep-alive` | 默认配置 |

## 3. 心跳与 Event 类型的对应关系

**IdeBridge 的 `: ping\n\n` 无法映射到 `types.rs` 中的 `Event` 枚举。**

- `: ping\n\n` 是 SSE 规范中的**注释行**，`eventsource_stream` 解析时会静默丢弃
- `Event::Unknown`（`types.rs:1040-1042`）捕获的是**未识别的事件类型名**（如 `event: server.heartbeat`），不是注释
- 所以客户端 `EventStream` **不会为心跳产生任何事件**，但心跳确实在传输层保持了连接活跃

## 4. 优化方案

### 4.1 优化一：reqwest Client 配置

**文件**：`src/types/client.rs`（注意：此文件为自动生成，可能需要在上层覆盖）

**问题**：`Client::builder().build()` 默认配置不适合 SSE 长连接。

**方案**：构建 `OpencodeClient` 时传入定制化的 `reqwest::Client`：

```rust
use std::time::Duration;
use reqwest::Client;

let client = Client::builder()
    // TCP 层 keepalive，防止连接被中间网络设备超时关闭
    .tcp_keepalive(Duration::from_secs(15))
    // SSE 是无限流，不应被连接池回收
    .pool_idle_timeout(None)
    // 禁用全局请求超时（SSE 连接需要无限期保持）
    .no_proxy()
    .build()?;

let opencode_client = OpencodeClient::with_client("http://127.0.0.1:50065", client)?;
```

> **注意**：`client.rs` 是自动生成代码，`with_client()` 方法已存在（行 44-47），
> 可以直接使用，无需修改生成代码。

### 4.2 优化二：SSE 流超时检测

**文件**：`src/bin/example.rs` 的 `sse_listener` 函数

**问题**：`stream.next().await` 无超时，连接已断但 TCP 未感知时会永远阻塞。

**方案**：用 `tokio::time::timeout` 包裹每次读取，超时即认为断连：

```rust
use std::time::Duration;

/// 心跳间隔 15 秒 × 3 = 45 秒超时（允许 2 次丢包容错）
const SSE_READ_TIMEOUT: Duration = Duration::from_secs(45);

async fn run_sse_stream(client: &OpencodeClient) -> Result<()> {
    let resp = client
        .event_subscribe(EventSubscribeRequest::default())
        .await
        .context("failed to subscribe to events")?;

    let EventSubscribeResponse::Ok(mut stream) = resp else {
        bail!("unexpected response from event subscription");
    };

    println!("Connected: listening for SSE events...");
    let mut delta_buf: HashMap<String, String> = HashMap::new();

    loop {
        match tokio::time::timeout(SSE_READ_TIMEOUT, stream.next()).await {
            Ok(Some(Ok(event))) => {
                // 正常处理事件（保留现有的 match event 逻辑）
                handle_event(event, &mut delta_buf);
            }
            Ok(Some(Err(e))) => {
                eprintln!("Event parse error: {}", e);
            }
            Ok(None) => {
                // 流正常结束（服务端主动关闭连接）
                println!("SSE stream ended normally");
                return Ok(());
            }
            Err(_) => {
                // 超时：45 秒内未收到任何数据（含心跳）
                bail!("SSE read timeout ({}s without data)", SSE_READ_TIMEOUT.as_secs());
            }
        }
    }
}
```

### 4.3 优化三：自动重连（指数退避）

**文件**：`src/bin/example.rs`，替换原有的 `sse_listener` 函数

**问题**：连接断开后直接退出，无重连。

**方案**：外层循环 + 指数退避：

```rust
const INITIAL_BACKOFF: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(30);

async fn sse_listener(client: &OpencodeClient) -> Result<()> {
    let mut backoff = INITIAL_BACKOFF;

    loop {
        match run_sse_stream(client).await {
            Ok(()) => {
                // 流正常结束（如服务端重启），立即重连
                println!("SSE stream closed, reconnecting immediately...");
                backoff = INITIAL_BACKOFF;
            }
            Err(e) => {
                eprintln!("SSE error: {}, retrying in {:?}...", e, backoff);
                tokio::time::sleep(backoff).await;
                // 指数退避：1s → 2s → 4s → 8s → 16s → 30s（封顶）
                backoff = (backoff * 2).min(MAX_BACKOFF);
            }
        }
    }
}
```

### 4.4 优化四：事件处理提取

**文件**：`src/bin/example.rs`

**问题**：事件处理逻辑嵌套在 SSE 循环中，重连后需要重复。

**方案**：提取为独立函数：

```rust
fn handle_event(event: Event, delta_buf: &mut HashMap<String, String>) {
    match event {
        Event::MessagePartDelta(d) => {
            delta_buf
                .entry(d.properties.part_id.clone())
                .or_default()
                .push_str(&d.properties.delta);
        }
        Event::SessionIdle(_) => {
            if !delta_buf.is_empty() {
                for (_, text) in delta_buf.drain() {
                    println!("Response:\n{}", text);
                }
            }
            println!("--- Ready for next input ---");
        }
        Event::SessionStatus(s) => {
            println!("[Status: {:?}]", s.properties.status);
        }
        Event::PermissionAsked(p) => {
            println!("[Permission requested: {:?}]", p.properties);
        }
        Event::QuestionAsked(q) => {
            println!("[Question: {:?}]", q.properties);
        }
        Event::SessionError(e) => {
            eprintln!("[Session error: {:?}]", e.properties.error);
        }
        Event::Unknown => {
            // 可能是 server.heartbeat 等未知事件类型，静默忽略
        }
        _ => {}
    }
}
```

## 5. 完整改造后的 example.rs 骨架

```rust
//! Example: Connect to OpenCode server with SSE auto-reconnect

use anyhow::{Context, Result, bail};
use futures::StreamExt;
use opencode_api::types::*;
use std::collections::HashMap;
use std::time::Duration;
use tokio::io::{self, AsyncBufReadExt, BufReader};

const SSE_READ_TIMEOUT: Duration = Duration::from_secs(45);
const INITIAL_BACKOFF: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(30);

#[tokio::main]
async fn main() -> Result<()> {
    // 使用定制化 reqwest Client，配置 TCP keepalive
    let http_client = reqwest::Client::builder()
        .tcp_keepalive(Duration::from_secs(15))
        .pool_idle_timeout(None)
        .build()
        .context("failed to build HTTP client")?;

    let client = OpencodeClient::with_client("http://127.0.0.1:50065", http_client)
        .context("failed to create OpenCode client")?;

    // 创建会话
    let session_resp = client
        .session_create(SessionCreateRequest {
            query: SessionCreateRequestQuery::default(),
            body: Some(SessionRequestBody {
                title: Some("Example SSE Session".to_string()),
                ..Default::default()
            }),
        })
        .await
        .context("failed to create session")?;

    let SessionCreateResponse::Ok(session) = session_resp else {
        bail!("session creation returned unexpected response");
    };
    let session_id = session.id.clone();

    // 并发运行 SSE 监听（带重连）和 stdin 交互
    tokio::select! {
        res = sse_listener(&client) => {
            res.context("SSE listener exited")?;
        }
        res = interactive_input(&client, &session_id) => {
            res.context("stdin handler exited")?;
        }
    }

    Ok(())
}

/// SSE 监听入口：自动重连 + 指数退避
async fn sse_listener(client: &OpencodeClient) -> Result<()> {
    let mut backoff = INITIAL_BACKOFF;

    loop {
        match run_sse_stream(client).await {
            Ok(()) => {
                println!("SSE stream closed, reconnecting immediately...");
                backoff = INITIAL_BACKOFF;
            }
            Err(e) => {
                eprintln!("SSE error: {}, retrying in {:?}...", e, backoff);
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(MAX_BACKOFF);
            }
        }
    }
}

/// 单次 SSE 流：带超时检测
async fn run_sse_stream(client: &OpencodeClient) -> Result<()> {
    let resp = client
        .event_subscribe(EventSubscribeRequest::default())
        .await
        .context("failed to subscribe to events")?;

    let EventSubscribeResponse::Ok(mut stream) = resp else {
        bail!("unexpected response from event subscription");
    };

    println!("Connected: listening for SSE events...");
    let mut delta_buf: HashMap<String, String> = HashMap::new();

    loop {
        match tokio::time::timeout(SSE_READ_TIMEOUT, stream.next()).await {
            Ok(Some(Ok(event))) => handle_event(event, &mut delta_buf),
            Ok(Some(Err(e))) => eprintln!("Event parse error: {}", e),
            Ok(None) => return Ok(()),
            Err(_) => bail!("SSE timeout ({}s)", SSE_READ_TIMEOUT.as_secs()),
        }
    }
}

/// 事件处理（独立函数，便于重连后复用）
fn handle_event(event: Event, delta_buf: &mut HashMap<String, String>) {
    match event {
        Event::MessagePartDelta(d) => {
            delta_buf
                .entry(d.properties.part_id.clone())
                .or_default()
                .push_str(&d.properties.delta);
        }
        Event::SessionIdle(_) => {
            if !delta_buf.is_empty() {
                for (_, text) in delta_buf.drain() {
                    println!("Response:\n{}", text);
                }
            }
            println!("--- Ready for next input ---");
        }
        Event::SessionStatus(s) => println!("[Status: {:?}]", s.properties.status),
        Event::PermissionAsked(p) => println!("[Permission requested: {:?}]", p.properties),
        Event::QuestionAsked(q) => println!("[Question: {:?}]", q.properties),
        Event::SessionError(e) => eprintln!("[Session error: {:?}]", e.properties.error),
        _ => {}
    }
}

/// stdin 交互（保持不变）
async fn interactive_input(client: &OpencodeClient, session_id: &str) -> Result<()> {
    let stdin = io::stdin();
    let mut lines = BufReader::new(stdin).lines();

    println!("--- Interactive mode: type a line and press Enter (Ctrl+D to exit) ---");
    while let Some(line) = lines.next_line().await? {
        let text = line.trim();
        if text.is_empty() {
            continue;
        }
        let req = SessionPromptRequest {
            path: SessionPromptRequestPath {
                session_id: session_id.to_string(),
            },
            query: SessionPromptRequestQuery::default(),
            body: Some(MessageRequestBody {
                parts: vec![PartKind2::Text(TextPartInput {
                    text: text.to_string(),
                    r#type: "text".to_string(),
                    ..Default::default()
                })],
                ..Default::default()
            }),
        };
        match client.session_prompt(req).await {
            Ok(resp) => match resp.text_content() {
                Ok(content) => println!("Server response: {}", content),
                Err(e) => eprintln!("Failed to extract response text: {}", e),
            },
            Err(e) => eprintln!("Failed to send prompt: {}", e),
        }
    }
    Ok(())
}
```

## 6. 设计原则总结

| 原则 | 来源 | 应用 |
|---|---|---|
| 心跳间隔 × 3 = 超时阈值 | IdeBridge 15s 心跳 | 客户端设 45s 读超时 |
| 指数退避重连 | 业界最佳实践 | 1s → 2s → 4s → ... → 30s 封顶 |
| 正常结束立即重连 | IdeBridge 会话管理 | 服务端重启时快速恢复 |
| TCP keepalive | IdeBridge `Connection: keep-alive` | reqwest `tcp_keepalive(15s)` |
| 注释格式心跳不触发事件 | SSE 规范 | `: ping\n\n` 仅保活传输层 |
| 事件处理与连接管理分离 | 单一职责 | `handle_event` 独立函数 |
