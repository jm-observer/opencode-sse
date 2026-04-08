use sse_parser::error::ParseError;
use sse_parser::model::SsePayload;
use sse_parser::parser::Parser;

#[tokio::test]
async fn test_single_line_success() {
    let mut parser = Parser::new();
    let line = r#"data:{"id":"1","object":"test","created":12345,"model":"gpt"}"#;
    assert!(parser.parse_line(line).await.is_none());
    // End of event
    let result_opt = parser.parse_line("").await;
    let result = result_opt.expect("expected Some");
    let payload = result.expect("expected Ok");
    assert_eq!(payload.id, "1");
    assert_eq!(payload.object, "test");
    assert_eq!(payload.created, 12345);
    assert_eq!(payload.model, "gpt");
}

#[tokio::test]
async fn test_multi_line_data() {
    let mut parser = Parser::new();
    // Split JSON across two data lines (no spaces needed)
    let line1 = "data:{\"id\":\"2\",\"object\":\"multi\"";
    let line2 = "data:,\"created\":67890,\"model\":\"gpt2\"}";
    assert!(parser.parse_line(line1).await.is_none());
    assert!(parser.parse_line(line2).await.is_none());
    let result = parser.parse_line("").await.expect("expected Some").expect("ok");
    assert_eq!(result.id, "2");
    assert_eq!(result.object, "multi");
    assert_eq!(result.created, 67890);
    assert_eq!(result.model, "gpt2");
}

#[tokio::test]
async fn test_invalid_json() {
    let mut parser = Parser::new();
    let line = "data:{invalid json}";
    assert!(parser.parse_line(line).await.is_none());
    let result = parser.parse_line("").await.expect("expected Some");
    match result {
        Err(ParseError::Json(_)) => {}
        _ => panic!("expected Json error"),
    }
}

#[tokio::test]
async fn test_empty_event() {
    let mut parser = Parser::new();
    let result = parser.parse_line("").await;
    assert!(result.is_none());
}

#[tokio::test]
async fn test_reset() {
    let mut parser = Parser::new();
    let line = r#"data:{"id":"3","object":"reset","created":1,"model":"gpt"}"#;
    parser.parse_line(line).await;
    let _ = parser.parse_line("").await; // consume first event
    parser.reset();
    // parse a second event
    let line2 = r#"data:{"id":"4","object":"second","created":2,"model":"gpt2"}"#;
    parser.parse_line(line2).await;
    let result = parser.parse_line("").await.expect("Some").expect("Ok");
    assert_eq!(result.id, "4");
}
