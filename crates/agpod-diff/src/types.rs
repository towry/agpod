//! Type definitions for diff processing

/// Represents a single file change in a git diff
#[derive(Debug)]
pub struct FileChange {
    pub old_path: Option<String>,
    pub new_path: Option<String>,
    pub change_type: ChangeType,
    pub content_lines: Vec<String>,
    pub is_large: bool,
}

/// Type of change detected in a git diff
#[derive(Debug)]
pub enum ChangeType {
    Added,
    Deleted,
    Modified,
    Renamed,
}

impl ChangeType {
    pub fn as_str(&self) -> &str {
        match self {
            ChangeType::Added => "added",
            ChangeType::Deleted => "deleted",
            ChangeType::Modified => "modified",
            ChangeType::Renamed => "renamed",
        }
    }
}
