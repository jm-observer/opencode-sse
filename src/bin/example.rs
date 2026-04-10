//! Example: Connect to OpenCode server, subscribe to SSE events, and interact via stdin.

use anyhow::{Context, Result};
use futures::StreamExt;
use opencode_api::types::{
    EventSubscribeRequest, EventSubscribeResponse, MessageRequestBody, OpencodeClient, PartKind2, SessionCreateRequest,
    SessionCreateRequestQuery, SessionCreateResponse, SessionPromptRequest, SessionRequestBody, TextPartInput,
};
use tokio::io::{self, AsyncBufReadExt, BufReader};

const DEFAULT_SERVER: &str = "127.0.0.1:4097";

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize client with configurable server address
    let server_addr = std::env::var("OPENCODE_SERVER").unwrap_or_else(|_| DEFAULT_SERVER.to_string());
    let client = OpencodeClient::with_base_url(&server_addr).context("failed to create Opencode client")?;

    // Create a session for prompting
    let session_resp = client
        .session_create(SessionCreateRequest {
            query: SessionCreateRequestQuery::default(),
            body: Some(SessionRequestBody {
                parent_id: None,
                permission: None,
                title: Some("Example SSE Session".to_string()),
                workspace_id: None,
            }),
        })
        .await
        .context("failed to create session")?;
    let SessionCreateResponse::Ok(session) = session_resp else {
        anyhow::bail!("session creation returned unexpected response");
    };
    let session_id = session.id.clone();

    // Run SSE listener and stdin handler concurrently
    tokio::select! {
        sse_res = sse_listener(&client) => {
            sse_res.context("SSE listener exited with error")?;
        }
        input_res = interactive_input(&client, &session_id) => {
            input_res.context("stdin handler exited with error")?;
        }
    }

    Ok(())
}

/// Subscribe to SSE events and print them
async fn sse_listener(client: &OpencodeClient) -> Result<()> {
    let req = EventSubscribeRequest::default();
    let resp = client
        .event_subscribe(req)
        .await
        .context("failed to subscribe to events")?;

    match resp {
        EventSubscribeResponse::Ok(mut stream) => {
            println!("Connected: listening for SSE events...");
            while let Some(event) = stream.next().await {
                match event {
                    Ok(ev) => println!("Event: {:?}", ev),
                    Err(e) => eprintln!("Event parse error: {}", e),
                }
            }
        }
        EventSubscribeResponse::Unknown => {
            anyhow::bail!("received unknown response from event subscription");
        }
    }
    Ok(())
}

/// Read lines from stdin and send them as prompts to the server
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
            path: opencode_api::types::SessionPromptRequestPath {
                session_id: session_id.to_string(),
            },
            query: opencode_api::types::SessionPromptRequestQuery::default(),
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
