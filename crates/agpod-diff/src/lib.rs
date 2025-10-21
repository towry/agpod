//! Git diff minimization for LLM context optimization
//!
//! This module provides functionality to minimize git diffs for efficient
//! token usage in Large Language Model contexts. It intelligently summarizes
//! large files while preserving essential change information.

mod processor;
mod save;
mod types;

// Public API - only export what's needed by main.rs
pub use processor::process_git_diff;

// Re-export for library users (allow unused since these are library APIs)
#[allow(unused_imports)]
pub use processor::{
    format_deleted_file_summary, format_large_file_summary, format_regular_file_diff,
    minimize_diff, parse_git_diff, remove_excessive_empty_lines,
};
#[allow(unused_imports)]
pub use save::{
    compute_file_hash, generate_chunk_suffix, get_project_identifier, parse_existing_review,
    save_diff_chunks, ReviewEntry,
};
#[allow(unused_imports)]
pub use types::{ChangeType, FileChange};

#[cfg(test)]
mod tests;
