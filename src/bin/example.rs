//! Example: Connect to Claude Code server with SSE auto-reconnect

use anyhow::{Context, Result, bail};
use futures::StreamExt;
use opencode_api::types::*;
use std::collections::HashMap;
use std::time::Duration;
use tokio::io::{self, AsyncBufReadExt, BufReader};
use tokio::time;

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
        .context("failed to create Claude Code client")?;

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
                time::sleep(backoff).await;
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
        match time::timeout(SSE_READ_TIMEOUT, stream.next()).await {
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
