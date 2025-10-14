use clap::{Parser, Subcommand};
use regex::Regex;
use std::collections::hash_map::DefaultHasher;
use std::env;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::{self, Read, Write};
use std::path::Path;
use std::process::Command;

mod kiro;

#[derive(Parser)]
#[command(name = "agpod")]
#[command(version = env!("CARGO_PKG_VERSION"))]
#[command(about = env!("CARGO_PKG_DESCRIPTION"), long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Minimize git diff for LLM context (reads from stdin)
    Diff {
        /// Save diff chunks to separate files
        #[arg(long)]
        save: bool,

        /// Specify custom output directory
        #[arg(long)]
        save_path: Option<String>,
    },
    /// Kiro workflow commands for PR draft management
    Kiro(kiro::KiroArgs),
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Diff { save, save_path }) => {
            // Process git diff from stdin
            match process_git_diff(save, save_path) {
                Ok(()) => {}
                Err(e) => {
                    eprintln!("Error: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Some(Commands::Kiro(args)) => {
            if let Err(e) = kiro::run(args) {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        }
        None => {
            // No command provided, print help
            use clap::CommandFactory;
            let _ = Cli::command().print_help();
            println!(); // Add a newline after help
        }
    }
}

/// Expand environment variables and tilde in path
#[allow(dead_code)]
fn expand_path(path: &str) -> String {
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

/// Get the git repository name or current directory name as project identifier
fn get_project_identifier() -> String {
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
fn compute_file_hash(content: &str) -> String {
    let mut hasher = DefaultHasher::new();
    content.hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

#[derive(Debug)]
struct ReviewEntry {
    hash: String,
    status: String,
    comments: String,
}

/// Parse existing REVIEW.md file to extract file entries
fn parse_existing_review(content: &str) -> std::collections::HashMap<String, ReviewEntry> {
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

fn process_git_diff(save_mode: bool, save_path: Option<String>) -> io::Result<()> {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;

    if save_mode {
        let path = save_path.as_deref().unwrap_or("llm/diff");
        save_diff_chunks(&input, path)?;
    } else {
        let minimized_diff = minimize_diff(&input);
        print!("{}", minimized_diff);
    }

    Ok(())
}

#[derive(Debug)]
struct FileChange {
    old_path: Option<String>,
    new_path: Option<String>,
    change_type: ChangeType,
    content_lines: Vec<String>,
    is_large: bool,
}

#[derive(Debug)]
enum ChangeType {
    Added,
    Deleted,
    Modified,
    Renamed,
}

impl ChangeType {
    fn as_str(&self) -> &str {
        match self {
            ChangeType::Added => "added",
            ChangeType::Deleted => "deleted",
            ChangeType::Modified => "modified",
            ChangeType::Renamed => "renamed",
        }
    }
}

fn minimize_diff(diff_content: &str) -> String {
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

fn parse_git_diff(diff_content: &str) -> Vec<FileChange> {
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

fn generate_chunk_suffix(index: usize) -> String {
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

fn save_diff_chunks(diff_content: &str, output_dir: &str) -> io::Result<()> {
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

fn format_large_file_summary(file_change: &FileChange) -> String {
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

fn format_deleted_file_summary(file_change: &FileChange) -> String {
    let unknown_path = "unknown".to_string();
    let path = file_change
        .old_path
        .as_ref()
        .or(file_change.new_path.as_ref())
        .unwrap_or(&unknown_path);

    format!("Deleted file: {}\n", path)
}

fn format_regular_file_diff(file_change: &FileChange) -> String {
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

fn remove_excessive_empty_lines(lines: &[String]) -> Vec<String> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{LazyLock, Mutex};

    // Shared lock to prevent parallel execution of tests that write to REVIEW.md
    static REVIEW_MD_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    #[test]
    fn test_empty_input() {
        let result = minimize_diff("");
        assert_eq!(result, "");
    }

    #[test]
    fn test_deleted_file() {
        let diff = r#"diff --git a/test.txt b/test.txt
deleted file mode 100644
index 1234567..0000000
--- a/test.txt
+++ /dev/null
@@ -1,3 +0,0 @@
-Line 1
-Line 2
-Line 3"#;

        let result = minimize_diff(diff);
        assert!(result.contains("Deleted file: test.txt"));
        assert!(!result.contains("Line 1"));
    }

    #[test]
    fn test_small_added_file() {
        let diff = r#"diff --git a/new.txt b/new.txt
new file mode 100644
index 0000000..abcdefg
--- /dev/null
+++ b/new.txt
@@ -0,0 +1,3 @@
+New line 1
+New line 2
+New line 3"#;

        let result = minimize_diff(diff);
        assert!(result.contains("diff --git a/new.txt b/new.txt"));
        assert!(result.contains("+New line 1"));
        assert!(result.contains("+New line 2"));
        assert!(result.contains("+New line 3"));
    }

    #[test]
    fn test_large_added_file() {
        // Read the actual large JSON file
        let json_content = include_str!("../test_data/large_config.json");

        // Create a git diff for adding this large JSON file
        let mut diff = String::from(
            r#"diff --git a/config/enterprise.json b/config/enterprise.json
new file mode 100644
index 0000000..1234567
--- /dev/null
+++ b/config/enterprise.json
"#,
        );

        // Add the JSON content as additions (with + prefix)
        let lines: Vec<&str> = json_content.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            diff.push_str(&format!("@@ -{},0 +{},1 @@\n", i + 1, i + 1));
            diff.push_str(&format!("+{}\n", line));
        }

        let result = minimize_diff(&diff);

        // Should show large file summary, not content
        assert!(result.contains("Large file change: config/enterprise.json"));
        assert!(result.contains("Change type: added"));
        assert!(result.contains("Content lines:"));

        // Should NOT contain actual JSON content
        assert!(!result.contains("enterprise-api-service"));
        assert!(!result.contains("postgresql"));
        assert!(!result.contains("prometheus"));
        assert!(!result.contains("authentication"));
    }

    #[test]
    fn test_large_json_file_realistic() {
        // Test with a realistic large JSON configuration file
        let json_content = include_str!("../test_data/large_config.json");
        let lines: Vec<&str> = json_content.lines().collect();

        // Verify our test file is actually large enough
        assert!(lines.len() > 400, "Test JSON file should have >400 lines");

        // Create a proper git diff format
        let diff = format!(
            r#"diff --git a/config/production.json b/config/production.json
new file mode 100644
index 0000000..abcdef123456
--- /dev/null
+++ b/config/production.json
@@ -0,0 +1,{} @@
{}"#,
            lines.len(),
            lines
                .iter()
                .map(|line| format!("+{}", line))
                .collect::<Vec<_>>()
                .join("\n")
        );

        let result = minimize_diff(&diff);

        // Verify it's treated as a large file
        assert!(result.contains("Large file change: config/production.json"));
        assert!(result.contains("Change type: added"));

        // Verify content is not shown (token efficiency)
        assert!(!result.contains("\"application\""));
        assert!(!result.contains("\"database\""));
        assert!(!result.contains("\"authentication\""));
        assert!(!result.contains("\"monitoring\""));

        // Verify the content line count is reasonable
        assert!(result.contains("Content lines:"));

        // Should be much shorter than original
        assert!(
            result.len() < diff.len() / 10,
            "Minimized output should be much smaller"
        );
    }

    #[test]
    fn test_modified_file() {
        let diff = r#"diff --git a/modified.txt b/modified.txt
index xyz123..abc456 100644
--- a/modified.txt
+++ b/modified.txt
@@ -1,3 +1,4 @@
 Existing line 1
-Old line 2
+Modified line 2
 Existing line 3
+Added line 4"#;

        let result = minimize_diff(diff);
        assert!(result.contains("diff --git a/modified.txt b/modified.txt"));
        assert!(result.contains("-Old line 2"));
        assert!(result.contains("+Modified line 2"));
        assert!(result.contains("+Added line 4"));
    }

    #[test]
    fn test_remove_excessive_empty_lines() {
        let lines = vec![
            "Line 1".to_string(),
            "".to_string(),
            "".to_string(),
            "".to_string(),
            "".to_string(),
            "Line 2".to_string(),
        ];

        let result = remove_excessive_empty_lines(&lines);
        let empty_count = result.iter().filter(|line| line.trim().is_empty()).count();
        assert_eq!(empty_count, 2); // Should have at most 2 empty lines
        assert_eq!(result[0], "Line 1");
        assert_eq!(result[result.len() - 1], "Line 2");
    }

    #[test]
    fn test_change_type_strings() {
        assert_eq!(ChangeType::Added.as_str(), "added");
        assert_eq!(ChangeType::Deleted.as_str(), "deleted");
        assert_eq!(ChangeType::Modified.as_str(), "modified");
        assert_eq!(ChangeType::Renamed.as_str(), "renamed");
    }

    #[test]
    fn test_expand_path_tilde() {
        // Test tilde expansion
        if let Ok(home) = env::var("HOME") {
            assert_eq!(expand_path("~/test"), format!("{}/test", home));
            assert_eq!(expand_path("~"), home);
        }

        // Test path without tilde (should remain unchanged)
        assert_eq!(expand_path("/tmp/test"), "/tmp/test");
        assert_eq!(expand_path("relative/path"), "relative/path");
    }

    #[test]
    fn test_expand_path_env_vars() {
        // Set a test environment variable
        env::set_var("TEST_VAR", "/test/path");

        // Test environment variable expansion
        assert_eq!(expand_path("$TEST_VAR/subdir"), "/test/path/subdir");
        assert_eq!(
            expand_path("prefix/$TEST_VAR/suffix"),
            "prefix//test/path/suffix"
        );

        // Test with HOME variable (should exist)
        if let Ok(home) = env::var("HOME") {
            assert_eq!(expand_path("$HOME/work"), format!("{}/work", home));
        }

        // Test non-existent variable (should keep as-is)
        assert_eq!(
            expand_path("$NONEXISTENT_VAR/path"),
            "$NONEXISTENT_VAR/path"
        );

        // Clean up
        env::remove_var("TEST_VAR");
    }

    #[test]
    fn test_expand_path_combined() {
        // Test combination of tilde and env var
        if let Ok(home) = env::var("HOME") {
            env::set_var("TEST_DIR", "mydir");
            assert_eq!(
                expand_path("~/work/$TEST_DIR"),
                format!("{}/work/mydir", home)
            );
            env::remove_var("TEST_DIR");
        }
    }

    #[test]
    fn test_renamed_file() {
        let diff = r#"diff --git a/old_name.txt b/new_name.txt
similarity index 100%
rename from old_name.txt
rename to new_name.txt"#;

        let result = minimize_diff(diff);
        assert!(result.contains("diff --git a/old_name.txt b/new_name.txt"));
    }

    #[test]
    fn test_multiple_files() {
        let diff = r#"diff --git a/deleted.txt b/deleted.txt
deleted file mode 100644
index 1234567..0000000
--- a/deleted.txt
+++ /dev/null
@@ -1,2 +0,0 @@
-Line 1
-Line 2

diff --git a/added.txt b/added.txt
new file mode 100644
index 0000000..abcdefg
--- /dev/null
+++ b/added.txt
@@ -0,0 +1,2 @@
+New line 1
+New line 2"#;

        let result = minimize_diff(diff);
        assert!(result.contains("Deleted file: deleted.txt"));
        assert!(result.contains("diff --git a/added.txt b/added.txt"));
        assert!(result.contains("+New line 1"));
    }

    #[test]
    fn test_generate_chunk_suffix() {
        assert_eq!(generate_chunk_suffix(0), "aa");
        assert_eq!(generate_chunk_suffix(1), "ab");
        assert_eq!(generate_chunk_suffix(25), "az");
        assert_eq!(generate_chunk_suffix(26), "ba");
        assert_eq!(generate_chunk_suffix(675), "zz");
        assert_eq!(generate_chunk_suffix(676), "0000");
        assert_eq!(generate_chunk_suffix(677), "0001");
    }

    #[test]
    fn test_save_diff_chunks() {
        use std::fs;
        use std::path::Path;

        // Use shared lock to prevent parallel execution of tests that write to REVIEW.md
        let _guard = REVIEW_MD_LOCK.lock().unwrap();

        let diff = r#"diff --git a/file1.txt b/file1.txt
new file mode 100644
index 0000000..abcdefg
--- /dev/null
+++ b/file1.txt
@@ -0,0 +1,2 @@
+Line 1
+Line 2

diff --git a/file2.txt b/file2.txt
new file mode 100644
index 0000000..xyz123
--- /dev/null
+++ b/file2.txt
@@ -0,0 +1,1 @@
+Content"#;

        // Clean up before test
        let _ = fs::remove_dir_all("llm/diff");

        // Test save with default path
        save_diff_chunks(diff, "llm/diff").unwrap();

        // For default path, no project subfolder is added
        let project_dir = "llm/diff";

        // Verify directory exists
        assert!(Path::new(&project_dir).exists());

        // Verify chunk files exist
        assert!(Path::new(&format!("{}/chunk_aa.diff", project_dir)).exists());
        assert!(Path::new(&format!("{}/chunk_ab.diff", project_dir)).exists());

        // Verify content
        let chunk_aa = fs::read_to_string(format!("{}/chunk_aa.diff", project_dir)).unwrap();
        assert!(chunk_aa.contains("diff --git a/file1.txt b/file1.txt"));
        assert!(chunk_aa.contains("+Line 1"));

        let chunk_ab = fs::read_to_string(format!("{}/chunk_ab.diff", project_dir)).unwrap();
        assert!(chunk_ab.contains("diff --git a/file2.txt b/file2.txt"));
        assert!(chunk_ab.contains("+Content"));

        // Verify REVIEW.md exists in the chunks directory and has correct format
        let review_path = format!("{}/REVIEW.md", project_dir);
        assert!(Path::new(&review_path).exists());
        let review = fs::read_to_string(review_path).unwrap();
        assert!(review.contains("# Code Review Tracking"));
        assert!(review.contains("## file1.txt"));
        assert!(review.contains("## file2.txt"));
        assert!(review.contains("meta:hash:"));
        assert!(review.contains("meta:diff_chunk: chunk_aa.diff"));
        assert!(review.contains("meta:diff_chunk: chunk_ab.diff"));
        assert!(review.contains("meta:status: pending"));

        // Clean up after test
        let _ = fs::remove_dir_all("llm/diff");
    }

    #[test]
    fn test_save_diff_chunks_custom_path() {
        use std::fs;
        use std::path::Path;

        // Use shared lock to prevent parallel execution of tests that write to REVIEW.md
        let _guard = REVIEW_MD_LOCK.lock().unwrap();

        let diff = r#"diff --git a/test.txt b/test.txt
new file mode 100644
index 0000000..abc123
--- /dev/null
+++ b/test.txt
@@ -0,0 +1,1 @@
+Test content"#;

        let custom_path = "custom/output";

        // Clean up before test
        let _ = fs::remove_dir_all(custom_path);

        // Test save with custom path
        save_diff_chunks(diff, custom_path).unwrap();

        // For relative paths, no project subfolder is added
        let project_dir = custom_path;

        // Verify directory exists
        assert!(Path::new(&project_dir).exists());

        // Verify chunk file exists
        assert!(Path::new(&format!("{}/chunk_aa.diff", project_dir)).exists());

        // Verify content
        let chunk_aa = fs::read_to_string(format!("{}/chunk_aa.diff", project_dir)).unwrap();
        assert!(chunk_aa.contains("diff --git a/test.txt b/test.txt"));
        assert!(chunk_aa.contains("+Test content"));

        // Verify REVIEW.md exists in the chunks directory
        let review_path = format!("{}/REVIEW.md", project_dir);
        assert!(Path::new(&review_path).exists());
        let review = fs::read_to_string(review_path).unwrap();
        assert!(review.contains("## test.txt"));
        assert!(review.contains("meta:diff_chunk: chunk_aa.diff"));

        // Clean up after test
        let _ = fs::remove_dir_all(custom_path);
    }

    #[test]
    fn test_get_project_identifier() {
        // Should return a non-empty string
        let project_id = get_project_identifier();
        assert!(!project_id.is_empty());
        // In git repository, should return "agpod"
        // In non-git context, returns current directory name or "default-project"
    }

    #[test]
    fn test_compute_file_hash() {
        let content1 = "Hello, World!";
        let content2 = "Hello, World!";
        let content3 = "Different content";

        // Same content should produce same hash
        assert_eq!(compute_file_hash(content1), compute_file_hash(content2));

        // Different content should produce different hash
        assert_ne!(compute_file_hash(content1), compute_file_hash(content3));
    }

    #[test]
    fn test_review_md_format() {
        use std::fs;
        use std::path::Path;

        // Use shared lock to prevent parallel execution of tests that write to REVIEW.md
        let _guard = REVIEW_MD_LOCK.lock().unwrap();

        let diff = r#"diff --git a/example.rs b/example.rs
new file mode 100644
index 0000000..abc123
--- /dev/null
+++ b/example.rs
@@ -0,0 +1,3 @@
+fn main() {
+    println!("Hello");
+}"#;

        // Clean up before test
        let _ = fs::remove_dir_all("test_review");

        // Save diff chunks
        save_diff_chunks(diff, "test_review").unwrap();

        // Verify REVIEW.md format in the chunks directory
        let review_path = "test_review/REVIEW.md";
        assert!(Path::new(review_path).exists());
        let review = fs::read_to_string(review_path).unwrap();

        // Check for header and guidelines
        assert!(review.contains("# Code Review Tracking"));
        assert!(review.contains("## Guidelines"));
        assert!(review.contains("Update `meta:status` after reviewing"));

        // Check for file entry
        assert!(review.contains("## example.rs"));
        assert!(review.contains("- meta:hash:"));
        assert!(review.contains("- meta:diff_chunk: chunk_aa.diff"));
        assert!(review.contains("- meta:status: pending"));
        assert!(review.contains("<!-- Review comments go here -->"));

        // Clean up
        let _ = fs::remove_dir_all("test_review");
    }

    #[test]
    fn test_review_md_persists_across_runs() {
        use std::fs;
        use std::path::Path;

        // Use shared lock to prevent parallel execution of tests that write to REVIEW.md
        let _guard = REVIEW_MD_LOCK.lock().unwrap();

        let diff1 = r#"diff --git a/file1.txt b/file1.txt
new file mode 100644
index 0000000..abc123
--- /dev/null
+++ b/file1.txt
@@ -0,0 +1,1 @@
+Content 1"#;

        let diff2 = r#"diff --git a/file2.txt b/file2.txt
new file mode 100644
index 0000000..xyz789
--- /dev/null
+++ b/file2.txt
@@ -0,0 +1,1 @@
+Content 2"#;

        let test_path = "test_persist";

        // Clean up before test
        let _ = fs::remove_dir_all(test_path);

        // First run - save file1
        save_diff_chunks(diff1, test_path).unwrap();

        let review_path = format!("{}/REVIEW.md", test_path);
        assert!(Path::new(&review_path).exists());

        // Modify the REVIEW.md by adding a comment to file1
        let mut review_content = fs::read_to_string(review_path.clone()).unwrap();
        review_content = review_content.replace(
            "<!-- Review comments go here -->",
            "This is a test comment for file1.txt",
        );
        review_content = review_content.replace(
            "- meta:status: pending",
            "- meta:status: reviewed@2025-01-01",
        );
        fs::write(&review_path, &review_content).unwrap();

        // Second run - save file2 (different file)
        save_diff_chunks(diff2, test_path).unwrap();

        // Verify REVIEW.md still exists
        assert!(Path::new(&review_path).exists());
        let final_review = fs::read_to_string(review_path).unwrap();

        // file1.txt should NOT be present (removed because not in current diff)
        assert!(!final_review.contains("## file1.txt"));

        // file2.txt should be present with pending status (new file)
        assert!(final_review.contains("## file2.txt"));
        assert!(final_review.contains("meta:status: pending"));
        assert!(final_review.contains("meta:diff_chunk: chunk_aa.diff"));

        // Clean up
        let _ = fs::remove_dir_all(test_path);
    }

    #[test]
    fn test_review_md_preserves_comments_on_hash_match() {
        use std::fs;

        // Use shared lock to prevent parallel execution of tests that write to REVIEW.md
        let _guard = REVIEW_MD_LOCK.lock().unwrap();

        let diff = r#"diff --git a/stable.txt b/stable.txt
new file mode 100644
index 0000000..abc123
--- /dev/null
+++ b/stable.txt
@@ -0,0 +1,1 @@
+Stable content"#;

        let test_path = "test_preserve";

        // Clean up before test
        let _ = fs::remove_dir_all(test_path);

        // First run
        save_diff_chunks(diff, test_path).unwrap();

        let review_path = format!("{}/REVIEW.md", test_path);

        // Modify the REVIEW.md
        let mut review_content = fs::read_to_string(review_path.clone()).unwrap();
        review_content = review_content.replace(
            "<!-- Review comments go here -->",
            "Important review notes\nMultiple lines of comments",
        );
        review_content = review_content.replace(
            "- meta:status: pending",
            "- meta:status: reviewed@2025-01-15",
        );
        fs::write(&review_path, &review_content).unwrap();

        // Second run with the same diff (hash should match)
        save_diff_chunks(diff, test_path).unwrap();

        // Verify comments and status are preserved
        let final_review = fs::read_to_string(review_path).unwrap();
        assert!(final_review.contains("Important review notes"));
        assert!(final_review.contains("Multiple lines of comments"));
        assert!(final_review.contains("- meta:status: reviewed@2025-01-15"));

        // Clean up
        let _ = fs::remove_dir_all(test_path);
    }

    #[test]
    fn test_review_md_marks_outdated_on_hash_change() {
        use std::fs;

        // Use shared lock to prevent parallel execution of tests that write to REVIEW.md
        let _guard = REVIEW_MD_LOCK.lock().unwrap();

        let diff1 = r#"diff --git a/changing.txt b/changing.txt
new file mode 100644
index 0000000..abc123
--- /dev/null
+++ b/changing.txt
@@ -0,0 +1,1 @@
+Original content"#;

        let diff2 = r#"diff --git a/changing.txt b/changing.txt
new file mode 100644
index 0000000..xyz789
--- /dev/null
+++ b/changing.txt
@@ -0,0 +1,1 @@
+Modified content"#;

        let test_path = "test_outdated";

        // Clean up before test
        let _ = fs::remove_dir_all(test_path);

        // First run
        save_diff_chunks(diff1, test_path).unwrap();

        let review_path = format!("{}/REVIEW.md", test_path);

        // Modify the REVIEW.md
        let mut review_content = fs::read_to_string(review_path.clone()).unwrap();
        review_content =
            review_content.replace("<!-- Review comments go here -->", "My review comments");
        review_content = review_content.replace(
            "- meta:status: pending",
            "- meta:status: reviewed@2025-01-20",
        );
        fs::write(&review_path, &review_content).unwrap();

        // Second run with modified diff (hash will change)
        save_diff_chunks(diff2, test_path).unwrap();

        // Verify status is marked as outdated but comments are preserved
        let final_review = fs::read_to_string(review_path).unwrap();
        assert!(final_review.contains("My review comments"));
        assert!(final_review.contains("- meta:status: outdated"));
        assert!(!final_review.contains("reviewed@2025-01-20"));

        // Clean up
        let _ = fs::remove_dir_all(test_path);
    }
}
