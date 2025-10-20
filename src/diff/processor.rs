//! Core diff processing and minimization logic

use super::types::{ChangeType, FileChange};
use regex::Regex;
use std::io::{self, Read};

/// Process git diff from stdin and output minimized version
pub fn process_git_diff(save_mode: bool, save_path: Option<String>) -> io::Result<()> {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;

    if save_mode {
        let path = save_path.as_deref().unwrap_or("llm/diff");
        super::save::save_diff_chunks(&input, path)?;
    } else {
        let minimized_diff = minimize_diff(&input);
        print!("{}", minimized_diff);
    }

    Ok(())
}

/// Minimize a git diff by summarizing large files and removing excessive empty lines
pub fn minimize_diff(diff_content: &str) -> String {
    let mut result = String::new();
    let file_changes = parse_git_diff(diff_content);

    for file_change in file_changes {
        match file_change.change_type {
            ChangeType::Deleted => {
                // For deleted files, only show metadata
                result.push_str(&format_deleted_file_summary(&file_change));
            }
            ChangeType::Added => {
                if file_change.is_large {
                    // Strategy 1: For large added files, only show metadata
                    result.push_str(&format_large_file_summary(&file_change));
                } else {
                    // For smaller added files, show the diff but remove excessive empty lines
                    result.push_str(&format_regular_file_diff(&file_change));
                }
            }
            _ => {
                // For modified and renamed files, apply the original logic
                if file_change.is_large {
                    result.push_str(&format_large_file_summary(&file_change));
                } else {
                    result.push_str(&format_regular_file_diff(&file_change));
                }
            }
        }
        result.push('\n');
    }

    result
}

/// Parse git diff content into structured file changes
pub fn parse_git_diff(diff_content: &str) -> Vec<FileChange> {
    let mut file_changes = Vec::new();
    let lines: Vec<&str> = diff_content.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        if let Some(file_change) = parse_file_change(&lines, &mut i) {
            file_changes.push(file_change);
        } else {
            i += 1;
        }
    }

    file_changes
}

fn parse_file_change(lines: &[&str], index: &mut usize) -> Option<FileChange> {
    let diff_header_re = Regex::new(r"^diff --git a/(.*?) b/(.*?)$").unwrap();

    if *index >= lines.len() {
        return None;
    }

    // Look for diff header
    let line = lines[*index];
    if let Some(captures) = diff_header_re.captures(line) {
        let old_path = captures.get(1).map(|m| m.as_str().to_string());
        let new_path = captures.get(2).map(|m| m.as_str().to_string());

        *index += 1;

        // Parse file metadata and determine change type
        let mut change_type = ChangeType::Modified;
        let mut content_lines = Vec::new();
        let mut total_changes = 0;

        // Collect all lines until next diff or end
        while *index < lines.len() && !lines[*index].starts_with("diff --git") {
            let line = lines[*index];

            // Determine change type from file mode lines
            if line.starts_with("new file mode") {
                change_type = ChangeType::Added;
            } else if line.starts_with("deleted file mode") {
                change_type = ChangeType::Deleted;
            } else if line.starts_with("rename from") || line.starts_with("rename to") {
                change_type = ChangeType::Renamed;
            }

            // Count actual content changes
            if (line.starts_with('+') && !line.starts_with("+++"))
                || (line.starts_with('-') && !line.starts_with("---"))
            {
                total_changes += 1;
            }

            content_lines.push(line.to_string());
            *index += 1;
        }

        // Determine if file is "large" (heuristic: more than 100 changes or 500 total lines)
        let is_large = total_changes > 100 || content_lines.len() > 500;

        return Some(FileChange {
            old_path,
            new_path,
            change_type,
            content_lines,
            is_large,
        });
    }

    None
}

/// Format a large file change as a summary
pub fn format_large_file_summary(file_change: &FileChange) -> String {
    let unknown_path = "unknown".to_string();
    let path = file_change
        .new_path
        .as_ref()
        .or(file_change.old_path.as_ref())
        .unwrap_or(&unknown_path);

    let mut summary = format!("Large file change: {}\n", path);
    summary.push_str(&format!(
        "Change type: {}\n",
        file_change.change_type.as_str()
    ));
    summary.push_str(&format!(
        "Content lines: {}\n",
        file_change.content_lines.len()
    ));

    summary
}

/// Format a deleted file as a summary
pub fn format_deleted_file_summary(file_change: &FileChange) -> String {
    let unknown_path = "unknown".to_string();
    let path = file_change
        .old_path
        .as_ref()
        .or(file_change.new_path.as_ref())
        .unwrap_or(&unknown_path);

    format!("Deleted file: {}\n", path)
}

/// Format a regular file change with full diff
pub fn format_regular_file_diff(file_change: &FileChange) -> String {
    let unknown_path = "unknown".to_string();
    let path = file_change
        .new_path
        .as_ref()
        .or(file_change.old_path.as_ref())
        .unwrap_or(&unknown_path);

    let mut result = format!(
        "diff --git a/{} b/{}\n",
        file_change.old_path.as_ref().unwrap_or(path),
        file_change.new_path.as_ref().unwrap_or(path)
    );

    // Remove excessive empty lines while preserving structure
    let cleaned_content = remove_excessive_empty_lines(&file_change.content_lines);

    for line in cleaned_content {
        result.push_str(&line);
        result.push('\n');
    }

    result
}

/// Remove excessive consecutive empty lines (keep max 2)
pub fn remove_excessive_empty_lines(lines: &[String]) -> Vec<String> {
    let mut result = Vec::new();
    let mut consecutive_empty = 0;

    for line in lines {
        let is_empty = line.trim().is_empty();

        if is_empty {
            consecutive_empty += 1;
            // Keep at most 2 consecutive empty lines
            if consecutive_empty <= 2 {
                result.push(line.clone());
            }
        } else {
            consecutive_empty = 0;
            result.push(line.clone());
        }
    }

    result
}
