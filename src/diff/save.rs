//! Diff chunk saving and review tracking functionality

use super::processor::parse_git_diff;
use regex::Regex;
use std::collections::hash_map::DefaultHasher;
use std::env;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::{self, Write};
use std::path::Path;
use std::process::Command;

/// Entry in REVIEW.md tracking file
#[derive(Debug)]
pub struct ReviewEntry {
    pub hash: String,
    pub status: String,
    pub comments: String,
}

/// Get the git repository name or current directory name as project identifier
pub fn get_project_identifier() -> String {
    // Try to get git repository name
    if let Ok(output) = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
    {
        if output.status.success() {
            if let Ok(path) = String::from_utf8(output.stdout) {
                let path = path.trim();
                if let Some(name) = Path::new(path).file_name() {
                    if let Some(name_str) = name.to_str() {
                        return name_str.to_string();
                    }
                }
            }
        }
    }

    // Fallback to current directory name
    if let Ok(current_dir) = env::current_dir() {
        if let Some(name) = current_dir.file_name() {
            if let Some(name_str) = name.to_str() {
                return name_str.to_string();
            }
        }
    }

    // Ultimate fallback
    "default-project".to_string()
}

/// Compute a simple hash of file content
pub fn compute_file_hash(content: &str) -> String {
    let mut hasher = DefaultHasher::new();
    content.hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

/// Parse existing REVIEW.md file to extract file entries
pub fn parse_existing_review(content: &str) -> std::collections::HashMap<String, ReviewEntry> {
    let mut entries = std::collections::HashMap::new();
    let mut current_file: Option<String> = None;
    let mut current_hash: Option<String> = None;
    let mut current_status: Option<String> = None;
    let mut current_comments = String::new();
    let mut in_comments = false;

    for line in content.lines() {
        if line.starts_with("## ") && !line.starts_with("## Guidelines") {
            // Save previous entry if exists
            if let (Some(file), Some(hash), Some(status)) = (
                current_file.take(),
                current_hash.take(),
                current_status.take(),
            ) {
                entries.insert(
                    file,
                    ReviewEntry {
                        hash,
                        status,
                        comments: current_comments.trim().to_string(),
                    },
                );
                current_comments.clear();
                in_comments = false;
            }

            // Start new entry
            current_file = Some(line[3..].trim().to_string());
        } else if current_file.is_some() {
            if let Some(stripped) = line.strip_prefix("- meta:hash: ") {
                current_hash = Some(stripped.trim().to_string());
            } else if let Some(stripped) = line.strip_prefix("- meta:status: ") {
                current_status = Some(stripped.trim().to_string());
                in_comments = true; // Comments come after status
            } else if line == "---" {
                // End of this file's section
                in_comments = false;
            } else if in_comments {
                // Collect comment lines (including empty lines, but not the placeholder)
                if !line.starts_with("- meta:") && line != "<!-- Review comments go here -->" {
                    if !current_comments.is_empty() {
                        current_comments.push('\n');
                    }
                    current_comments.push_str(line);
                }
            }
        }
    }

    // Save last entry if exists
    if let (Some(file), Some(hash), Some(status)) = (current_file, current_hash, current_status) {
        entries.insert(
            file,
            ReviewEntry {
                hash,
                status,
                comments: current_comments.trim().to_string(),
            },
        );
    }

    entries
}

/// Generate chunk suffix (aa-zz, then numbers)
pub fn generate_chunk_suffix(index: usize) -> String {
    // First use aa-zz (26*26 = 676 combinations)
    if index < 676 {
        let first = (index / 26) as u8;
        let second = (index % 26) as u8;
        format!("{}{}", (b'a' + first) as char, (b'a' + second) as char)
    } else {
        // After zz, use numbers
        format!("{:04}", index - 676)
    }
}

/// Expand environment variables and tilde in path
#[allow(dead_code)]
pub(crate) fn expand_path(path: &str) -> String {
    let mut expanded = path.to_string();

    // Expand tilde (~) to home directory
    if expanded.starts_with("~/") {
        if let Ok(home) = env::var("HOME") {
            expanded = expanded.replacen("~/", &format!("{}/", home), 1);
        }
    } else if expanded == "~" {
        if let Ok(home) = env::var("HOME") {
            expanded = home;
        }
    }

    // Expand environment variables like $HOME, $VAR, etc.
    let re = Regex::new(r"\$([A-Z_][A-Z0-9_]*)").unwrap();
    expanded = re
        .replace_all(&expanded, |caps: &regex::Captures| {
            let var_name = &caps[1];
            env::var(var_name).unwrap_or_else(|_| format!("${}", var_name))
        })
        .to_string();

    expanded
}

/// Save diff chunks to separate files with review tracking
pub fn save_diff_chunks(diff_content: &str, output_dir: &str) -> io::Result<()> {
    // Determine if we should add project identifier to path
    // Add project subfolder only for absolute paths (outside the project)
    // For relative paths, user is saving within their project, so no subfolder needed
    let is_relative_path = !output_dir.starts_with('/');
    let project_output_dir = if is_relative_path {
        // For relative paths, don't add project subfolder since we're already in the project
        output_dir.to_string()
    } else {
        // For absolute paths, add project identifier to prevent conflicts
        let project_id = get_project_identifier();
        format!("{}/{}", output_dir, project_id)
    };

    // Try to read existing REVIEW.md from the output directory BEFORE cleaning up
    let review_path = format!("{}/REVIEW.md", project_output_dir);
    let existing_review = fs::read_to_string(review_path.clone()).ok();
    let existing_entries = if let Some(content) = &existing_review {
        parse_existing_review(content)
    } else {
        std::collections::HashMap::new()
    };

    // Remove old chunk files but keep REVIEW.md
    if Path::new(&project_output_dir).exists() {
        // Read directory and remove only .diff files
        if let Ok(entries) = fs::read_dir(&project_output_dir) {
            for entry in entries.flatten() {
                if let Ok(file_type) = entry.file_type() {
                    if file_type.is_file() {
                        if let Some(path_str) = entry.path().to_str() {
                            if path_str.ends_with(".diff") {
                                let _ = fs::remove_file(entry.path());
                            }
                        }
                    }
                }
            }
        }
    } else {
        fs::create_dir_all(&project_output_dir)?;
    }

    let file_changes = parse_git_diff(diff_content);

    // Prepare REVIEW.md content
    let mut review_content = String::from(
        "# Code Review Tracking\n\n\
        This file tracks the review status of code changes.\n\n\
        ## Guidelines\n\
        - Diff chunks are stored in: ",
    );
    review_content.push_str(&project_output_dir);
    review_content.push_str(
        "/\n\
        - Update `meta:status` after reviewing each file\n\
        - Status values: `pending`, `reviewed@YYYY-MM-DD`, `outdated`\n\
        - If file hash changes on subsequent runs, status will be automatically set to `outdated`\n\
        - Add review comments in the placeholder section below each file\n\
        - On each run, file sections not present in current diff are removed\n\n\
        ---\n\n",
    );

    // Track which files are in the current diff
    let mut current_files = std::collections::HashSet::new();

    for (index, file_change) in file_changes.iter().enumerate() {
        let suffix = generate_chunk_suffix(index);
        let chunk_filename = format!("chunk_{}.diff", suffix);
        let chunk_path = format!("{}/{}", project_output_dir, chunk_filename);

        let unknown_path = "unknown".to_string();
        let filepath = file_change
            .new_path
            .as_ref()
            .or(file_change.old_path.as_ref())
            .unwrap_or(&unknown_path);

        current_files.insert(filepath.clone());

        let mut chunk_content = format!(
            "diff --git a/{} b/{}\n",
            file_change.old_path.as_ref().unwrap_or(filepath),
            file_change.new_path.as_ref().unwrap_or(filepath)
        );

        for line in &file_change.content_lines {
            chunk_content.push_str(line);
            chunk_content.push('\n');
        }

        // Compute hash of the chunk content
        let file_hash = compute_file_hash(&chunk_content);

        // Write chunk file
        let mut file = fs::File::create(&chunk_path)?;
        file.write_all(chunk_content.as_bytes())?;

        // Check if this file existed before
        let (status, comments) = if let Some(existing) = existing_entries.get(filepath) {
            // File existed before - check if hash changed
            if existing.hash == file_hash {
                // Hash unchanged - preserve status and comments
                (existing.status.clone(), existing.comments.clone())
            } else {
                // Hash changed - mark as outdated
                ("outdated".to_string(), existing.comments.clone())
            }
        } else {
            // New file - set as pending with no comments
            ("pending".to_string(), String::new())
        };

        // Add entry to REVIEW.md
        review_content.push_str(&format!("## {}\n", filepath));
        review_content.push_str(&format!("- meta:hash: {}\n", file_hash));
        review_content.push_str(&format!("- meta:diff_chunk: {}\n", chunk_filename));
        review_content.push_str(&format!("- meta:status: {}\n\n", status));

        if comments.is_empty() {
            review_content.push_str("<!-- Review comments go here -->\n\n");
        } else {
            review_content.push_str(&comments);
            review_content.push('\n');
        }

        review_content.push_str("---\n\n");
    }

    // Write REVIEW.md to the same directory as chunks
    let mut review_file = fs::File::create(&review_path)?;
    review_file.write_all(review_content.as_bytes())?;

    // Get absolute path for REVIEW.md
    let review_absolute_path = std::path::PathBuf::from(&project_output_dir)
        .join("REVIEW.md")
        .canonicalize()
        .ok()
        .and_then(|p| p.to_str().map(String::from))
        .unwrap_or_else(|| review_path.clone());

    // Output paths in machine-readable format to stdout
    println!("generated: {}/", project_output_dir);
    println!("REVIEW.md: {}", review_absolute_path);

    Ok(())
}
