use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::Path;

// ---------------------------------------------------------------------------
// Provider capability: system prompt support
// ---------------------------------------------------------------------------

/// How a provider accepts system prompts at the CLI level.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SystemPromptSupport {
    /// Provider does not support system prompts.
    None,
    /// Provider supports system prompts via inline text and/or file path flags.
    Supported {
        text_flag: Option<&'static str>,
        file_flag: Option<&'static str>,
    },
}

/// Declared capabilities of a hive provider.
#[derive(Debug, Clone)]
pub struct HiveProviderCaps {
    pub system_prompt: SystemPromptSupport,
}

/// Look up provider capabilities by command name.
///
/// Known providers get concrete caps; unknown providers default to no
/// system-prompt support (the config field is silently ignored).
pub fn provider_caps(provider_command: &str) -> HiveProviderCaps {
    let base = provider_command
        .rsplit('/')
        .next()
        .unwrap_or(provider_command);
    match base {
        // Claude Code CLI help advertises `--system-prompt[-file]`.
        "claude" | "claw" => HiveProviderCaps {
            system_prompt: SystemPromptSupport::Supported {
                text_flag: Some("--system-prompt"),
                file_flag: Some("--system-prompt-file"),
            },
        },
        _ => HiveProviderCaps {
            system_prompt: SystemPromptSupport::None,
        },
    }
}

/// Resolve a mode's `system_prompt` / `system_prompt_file` config into CLI
/// args, respecting the provider's declared capabilities.
///
/// - Both fields set → error (caller should have validated already, but
///   defence in depth).
/// - Neither set → empty vec.
/// - Text given + text-support → `[flag, text]`.
/// - Text given + file-only support → write temp file in `run_dir`, `[flag, path]`.
/// - File given + file-support → `[flag, expanded_path]`.
/// - File given + text-only support → read file content, `[flag, content]`.
/// - Provider is None → empty vec (silently ignored).
pub fn resolve_system_prompt_args(
    caps: &HiveProviderCaps,
    system_prompt: Option<&str>,
    system_prompt_file: Option<&str>,
    run_dir: &Path,
) -> Result<Vec<String>> {
    if system_prompt.is_some() && system_prompt_file.is_some() {
        return Err(anyhow!(
            "system_prompt and system_prompt_file are mutually exclusive"
        ));
    }

    let (text, file) = match (system_prompt, system_prompt_file) {
        (None, None) => return Ok(Vec::new()),
        (Some(t), None) => (Some(t), None),
        (None, Some(f)) => (None, Some(f)),
        _ => unreachable!(),
    };

    match &caps.system_prompt {
        SystemPromptSupport::None => Ok(Vec::new()),
        SystemPromptSupport::Supported {
            text_flag,
            file_flag,
        } => {
            if let Some(t) = text {
                if let Some(flag) = text_flag {
                    return Ok(vec![flag.to_string(), t.to_string()]);
                }
                if let Some(flag) = file_flag {
                    let tmp = run_dir.join("system_prompt.txt");
                    fs::write(&tmp, t).with_context(|| {
                        format!(
                            "failed to write system prompt temp file `{}`",
                            tmp.display()
                        )
                    })?;
                    return Ok(vec![flag.to_string(), tmp.display().to_string()]);
                }
            } else if let Some(f) = file {
                if let Some(flag) = file_flag {
                    let path = expand_home(f)?;
                    return Ok(vec![flag.to_string(), path.display().to_string()]);
                }
                if let Some(flag) = text_flag {
                    let path = expand_home(f)?;
                    let content = fs::read_to_string(&path).with_context(|| {
                        format!("failed to read system_prompt_file `{}`", path.display())
                    })?;
                    return Ok(vec![flag.to_string(), content]);
                }
            }

            Err(anyhow!(
                "provider declared invalid system prompt support: no usable flags configured"
            ))
        }
    }
}

fn expand_home(path: &str) -> Result<std::path::PathBuf> {
    if path == "~" {
        return dirs::home_dir().ok_or_else(|| anyhow!("failed to resolve home directory"));
    }
    if let Some(stripped) = path.strip_prefix("~/") {
        let home = dirs::home_dir().ok_or_else(|| anyhow!("failed to resolve home directory"))?;
        return Ok(home.join(stripped));
    }
    Ok(std::path::PathBuf::from(path))
}

// ---------------------------------------------------------------------------
// Provider output parsing (existing)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::tempdir;

    #[test]
    fn provider_caps_claude_supports_text_and_file_system_prompt() {
        let caps = provider_caps("claude");
        assert!(matches!(
            caps.system_prompt,
            SystemPromptSupport::Supported {
                text_flag: Some("--system-prompt"),
                file_flag: Some("--system-prompt-file"),
            }
        ));
    }

    #[test]
    fn provider_caps_claw_supports_text_and_file_system_prompt() {
        let caps = provider_caps("claw");
        assert!(matches!(
            caps.system_prompt,
            SystemPromptSupport::Supported { .. }
        ));
    }

    #[test]
    fn provider_caps_unknown_has_no_system_prompt() {
        let caps = provider_caps("some-unknown-provider");
        assert!(matches!(caps.system_prompt, SystemPromptSupport::None));
    }

    #[test]
    fn provider_caps_full_path_uses_basename() {
        let caps = provider_caps("/usr/local/bin/claude");
        assert!(matches!(
            caps.system_prompt,
            SystemPromptSupport::Supported { .. }
        ));
    }

    #[test]
    fn resolve_args_neither_field_returns_empty() {
        let caps = HiveProviderCaps {
            system_prompt: SystemPromptSupport::Supported {
                text_flag: Some("--system-prompt"),
                file_flag: None,
            },
        };
        let args =
            resolve_system_prompt_args(&caps, None, None, Path::new("/tmp")).expect("should ok");
        assert!(args.is_empty());
    }

    #[test]
    fn resolve_args_both_fields_errors() {
        let caps = HiveProviderCaps {
            system_prompt: SystemPromptSupport::Supported {
                text_flag: Some("--system-prompt"),
                file_flag: None,
            },
        };
        let result =
            resolve_system_prompt_args(&caps, Some("text"), Some("file"), Path::new("/tmp"));
        assert!(result.is_err());
    }

    #[test]
    fn resolve_args_text_with_text_provider() {
        let caps = HiveProviderCaps {
            system_prompt: SystemPromptSupport::Supported {
                text_flag: Some("--system-prompt"),
                file_flag: None,
            },
        };
        let args = resolve_system_prompt_args(&caps, Some("hello"), None, Path::new("/tmp"))
            .expect("should ok");
        assert_eq!(args, vec!["--system-prompt", "hello"]);
    }

    #[test]
    fn resolve_args_file_with_text_provider_reads_file() {
        let temp = tempdir().expect("temp dir");
        let file = temp.path().join("prompt.md");
        fs::write(&file, "from file").expect("write");
        let caps = HiveProviderCaps {
            system_prompt: SystemPromptSupport::Supported {
                text_flag: Some("--system-prompt"),
                file_flag: None,
            },
        };
        let args =
            resolve_system_prompt_args(&caps, None, Some(&file.display().to_string()), temp.path())
                .expect("should ok");
        assert_eq!(args, vec!["--system-prompt", "from file"]);
    }

    #[test]
    fn resolve_args_text_with_file_provider_writes_temp() {
        let temp = tempdir().expect("temp dir");
        let caps = HiveProviderCaps {
            system_prompt: SystemPromptSupport::Supported {
                text_flag: None,
                file_flag: Some("--sys-file"),
            },
        };
        let args = resolve_system_prompt_args(&caps, Some("inline text"), None, temp.path())
            .expect("should ok");
        assert_eq!(args[0], "--sys-file");
        let written = fs::read_to_string(&args[1]).expect("read temp file");
        assert_eq!(written, "inline text");
    }

    #[test]
    fn resolve_args_file_with_file_provider_passes_path() {
        let temp = tempdir().expect("temp dir");
        let file = temp.path().join("prompt.md");
        fs::write(&file, "content").expect("write");
        let caps = HiveProviderCaps {
            system_prompt: SystemPromptSupport::Supported {
                text_flag: None,
                file_flag: Some("--sys-file"),
            },
        };
        let args =
            resolve_system_prompt_args(&caps, None, Some(&file.display().to_string()), temp.path())
                .expect("should ok");
        assert_eq!(
            args,
            vec!["--sys-file".to_string(), file.display().to_string()]
        );
    }

    #[test]
    fn resolve_args_text_with_both_provider_uses_text_flag() {
        let caps = HiveProviderCaps {
            system_prompt: SystemPromptSupport::Supported {
                text_flag: Some("--sp"),
                file_flag: Some("--sp-file"),
            },
        };
        let args = resolve_system_prompt_args(&caps, Some("inline"), None, Path::new("/tmp"))
            .expect("should ok");
        assert_eq!(args, vec!["--sp", "inline"]);
    }

    #[test]
    fn resolve_args_file_with_both_provider_uses_file_flag() {
        let temp = tempdir().expect("temp dir");
        let file = temp.path().join("prompt.md");
        fs::write(&file, "content").expect("write");
        let caps = HiveProviderCaps {
            system_prompt: SystemPromptSupport::Supported {
                text_flag: Some("--sp"),
                file_flag: Some("--sp-file"),
            },
        };
        let args =
            resolve_system_prompt_args(&caps, None, Some(&file.display().to_string()), temp.path())
                .expect("should ok");
        assert_eq!(
            args,
            vec!["--sp-file".to_string(), file.display().to_string()]
        );
    }

    #[test]
    fn resolve_args_none_support_silently_ignores() {
        let caps = HiveProviderCaps {
            system_prompt: SystemPromptSupport::None,
        };
        let args = resolve_system_prompt_args(&caps, Some("hello"), None, Path::new("/tmp"))
            .expect("should ok");
        assert!(args.is_empty());
    }

    #[test]
    fn expand_home_expands_tilde() {
        let result = expand_home("~/test").expect("expand");
        assert!(result.is_absolute());
        assert!(result.ends_with("test"));
    }

    #[test]
    fn expand_home_leaves_absolute_path() {
        let result = expand_home("/absolute/path").expect("expand");
        assert_eq!(result, PathBuf::from("/absolute/path"));
    }
}
