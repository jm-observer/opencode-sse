pub mod error {
    use thiserror::Error;
    #[derive(Debug, Error)]
    pub enum ParseError {
        #[error("JSON error: {0}")]
        Json(serde_json::Error),
        #[error("Other error")]
        Other,
    }
}

pub mod model {
    use serde::Deserialize;
    #[derive(Debug, Deserialize)]
    pub struct SsePayload {
        pub id: String,
        pub object: String,
        pub created: i64,
        pub model: String,
    }
}

pub mod parser {
    use super::error::ParseError;
    use super::model::SsePayload;
    use serde_json::from_str;
    use std::future::Future;
    use std::pin::Pin;

    pub struct Parser {
        buffer: String,
    }

    impl Parser {
        pub fn new() -> Self {
            Self { buffer: String::new() }
        }
        pub fn reset(&mut self) {
            self.buffer.clear();
        }
        pub async fn parse_line(&mut self, line: &str) -> Option<Result<SsePayload, ParseError>> {
            if line.is_empty() {
                if self.buffer.is_empty() {
                    return None;
                }
                // End of event, parse accumulated buffer
                let json_str = self.buffer.trim_start_matches("data:");
                let res = from_str::<SsePayload>(json_str).map_err(ParseError::Json);
                self.buffer.clear();
                return Some(res);
            }
            // Accumulate data lines (strip leading "data:")
            let trimmed = line.trim_start_matches("data:");
            self.buffer.push_str(trimmed);
            None
        }
    }
}
