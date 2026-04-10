#![allow(clippy::all)]

// use std::collections::HashMap;
use crate::types::{
    Command200Response2, MessageRequestBody, OpencodeClient, Part, PartKind2, SessionCreateRequest,
    SessionCreateRequestQuery, SessionCreateResponse, SessionDeleteRequest, SessionDeleteRequestPath,
    SessionDeleteRequestQuery, SessionPromptRequest, SessionPromptRequestPath, SessionPromptRequestQuery,
    SessionPromptResponse, SessionRequestBody, TextPartInput,
};
use anyhow::{Result, anyhow, bail};
// use std::path::PathBuf;
// use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};

pub mod types;

#[allow(dead_code)]
async fn run_interactive_chat_v2() -> Result<()> {
    let client = OpencodeClient::with_base_url("http://127.0.0.1:33063")?;
    let resp = client
        .session_create(SessionCreateRequest {
            query: SessionCreateRequestQuery::default(),
            body: Some(SessionRequestBody {
                parent_id: None,
                permission: None,
                title: Some("Interactive Chat Mode".to_string()),
                workspace_id: None,
            }),
        })
        .await?;
    let SessionCreateResponse::Ok(session) = resp else {
        bail!("Session create request failed");
    };
    let session_id = session.id.clone();
    // 3. 设置异步标准输入读取
    let stdin = tokio::io::stdin();
    let mut reader = BufReader::new(stdin).lines();

    // 2. 统一监听与自愈机制 (Industry Best Practice: Automatic Re-subscription)
    println!("\n--- 进入实时回显模式 (包含思考过程，支持自动重连) ---");
    loop {
        // A. 处理用户输入
        let line_res = reader.next_line().await;
        match line_res {
            Ok(Some(line)) if !line.trim().is_empty() => match handle_chat_input_v2(&client, &session_id, line).await {
                Ok(res) => {
                    println!("response {res}");
                }
                Err(e) => {
                    eprintln!("发送指令失败: {:?}", e);
                }
            },
            Ok(Some(_)) => continue,
            Ok(None) | Err(_) => break,
        }
    }

    println!("\n[交互模式结束] 正在清理会话...");

    let resp = client
        .session_delete(SessionDeleteRequest {
            path: SessionDeleteRequestPath { session_id },
            query: SessionDeleteRequestQuery::default(),
        })
        .await?;
    println!("{resp:?}");
    Ok(())
}

/// 封装：处理用户输入并发送异步 Prompt
#[allow(dead_code)]
async fn handle_chat_input_v2(client: &types::OpencodeClient, session_id: &str, line: String) -> Result<String> {
    if line.trim().is_empty() {
        bail!("Empty line");
    }
    println!("  [发送] >> {}", line);
    let resp = client
        .session_prompt(SessionPromptRequest {
            path: SessionPromptRequestPath {
                session_id: session_id.to_string(),
            },
            query: SessionPromptRequestQuery {
                directory: None,
                workspace: None,
            },
            body: Some(MessageRequestBody {
                agent: None,
                format: None,
                message_id: None,
                model: None,
                no_reply: None,
                parts: vec![PartKind2::Text(TextPartInput {
                    id: None,
                    ignored: None,
                    metadata: None,
                    synthetic: None,
                    text: line,
                    time: None,
                    r#type: "text".to_string(),
                })],
                system: None,
                tools: None,
                variant: None,
            }),
        })
        .await?;
    resp.text_content()
}

impl Command200Response2 {
    pub fn text_content(&self) -> anyhow::Result<String> {
        if self.parts.iter().any(|x| {
            if let Part::Text(text) = x {
                text.r#type == "step-finish"
            } else {
                false
            }
        }) {
            if let Some(content) = self.parts.iter().find_map(|x| {
                if let Part::Text(text) = x {
                    if text.r#type == "text" {
                        return Some(text.text.clone());
                    }
                }
                None
            }) {
                return Ok(content);
            }
        }
        Err(anyhow!("could't find text content"))
    }
}

impl SessionPromptResponse {
    pub fn text_content(&self) -> anyhow::Result<String> {
        match self {
            SessionPromptResponse::Ok(res) => res.text_content(),
            SessionPromptResponse::BadRequest(err) => {
                bail!("BadRequest {err:?}");
            }
            SessionPromptResponse::NotFound(err) => {
                bail!("NotFound {err:?}");
            }
            SessionPromptResponse::Unknown => {
                bail!("Unknown");
            }
        }
    }
}
