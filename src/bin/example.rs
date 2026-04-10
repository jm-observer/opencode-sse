//! Example: Connect to 127.0.0.1:4096, subscribe to SSE events, read user input from stdin, and print result.

use anyhow::Result;
// futures imports removed as not needed
use opencode_api::types::{EventSubscribeRequest, OpencodeClient};
use tokio::io::{self, AsyncBufReadExt, BufReader};

#[tokio::main]
async fn main() -> Result<()> {
    // Use the OpencodeClient for HTTP requests

    // Use the generated client to subscribe to SSE events
    let openc_client = OpencodeClient::with_base_url("127.0.0.1:4097")?;
    let subscribe_req = EventSubscribeRequest::default();
    let response = openc_client.event_subscribe(subscribe_req).await?;
    // Assuming the response contains the necessary event stream or confirmation
    println!("Subscribed to events, response: {:?}", response);

    // Note: SSE handling is managed by OpencodeClient's event_subscribe method.
    // Additional event processing can be added here if needed.

    // Read user input from stdin and send it to the server via POST
    let stdin = io::stdin();
    let mut lines = BufReader::new(stdin).lines();

    println!("\n--- Interactive mode: type lines to send to server ---");
    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let resp = openc_client
            .client
            .post("http://127.0.0.1:4096")
            .body(line.clone())
            .send()
            .await?;
        println!("Sent line, server responded with status: {}", resp.status());
    }

    Ok(())
}
