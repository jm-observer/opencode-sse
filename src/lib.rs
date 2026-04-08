#![allow(clippy::all)]

use anyhow::{anyhow, bail};
use crate::types::{Command200Response2, Part, SessionPromptResponse};

pub mod types;

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
            SessionPromptResponse::Ok(res) => {
                res.text_content()
            }
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
