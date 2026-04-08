use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HiveProviderOutput {
    pub provider: String,
    pub format: HiveProviderOutputFormat,
    pub session_id: Option<String>,
    pub summary: Option<String>,
    #[serde(default)]
    pub json_keys: Vec<String>,
    pub parse_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HiveProviderOutputFormat {
    Json,
    Text,
    Unknown,
}

pub fn default_claude_provider() -> String {
    "claude".to_string()
}

pub fn parse_provider_output(
    provider: &str,
    output_path: &str,
    summarize: impl Fn(&str) -> String,
) -> HiveProviderOutput {
    let raw = match fs::read_to_string(output_path) {
        Ok(raw) => raw,
        Err(err) => {
            return HiveProviderOutput {
                provider: provider.to_string(),
                format: HiveProviderOutputFormat::Unknown,
                session_id: None,
                summary: None,
                json_keys: Vec::new(),
                parse_error: Some(err.to_string()),
            };
        }
    };

    match serde_json::from_str::<Value>(&raw) {
        Ok(Value::Object(obj)) => {
            let session_id = obj
                .get("session_id")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            let summary = obj
                .get("result")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .or_else(|| {
                    obj.get("summary")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned)
                })
                .or_else(|| session_id.as_ref().map(|id| format!("session_id={id}")));
            let mut json_keys = obj.keys().cloned().collect::<Vec<_>>();
            json_keys.sort();
            HiveProviderOutput {
                provider: provider.to_string(),
                format: HiveProviderOutputFormat::Json,
                session_id,
                summary,
                json_keys,
                parse_error: None,
            }
        }
        Ok(_) => HiveProviderOutput {
            provider: provider.to_string(),
            format: HiveProviderOutputFormat::Json,
            session_id: None,
            summary: Some(summarize(&raw)),
            json_keys: Vec::new(),
            parse_error: Some("provider output is valid json but not an object".to_string()),
        },
        Err(err) => HiveProviderOutput {
            provider: provider.to_string(),
            format: HiveProviderOutputFormat::Text,
            session_id: None,
            summary: Some(summarize(&raw)),
            json_keys: Vec::new(),
            parse_error: Some(err.to_string()),
        },
    }
}
