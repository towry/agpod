use regex::Regex;
use std::collections::hash_map::DefaultHasher;
use std::env;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::{self, Read, Write};
use std::path::Path;
use std::process::Command;

fn main() {
    let args: Vec<String> = env::args().collect();

    // Check for help or version flags
    if args.contains(&"--help".to_string()) || args.contains(&"-h".to_string()) {
        print_help();
        return;
    }

    if args.contains(&"--version".to_string()) || args.contains(&"-V".to_string()) {
        print_version();
        return;
    }

    let save_mode = args.contains(&"--save".to_string());

    // Parse optional --save-path argument
    let save_path = parse_save_path(&args);

    match process_git_diff(save_mode, save_path) {
        Ok(()) => {}
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    }
}

fn print_version() {
    println!("minimize-git-diff-llm {}", env!("CARGO_PKG_VERSION"));
}

fn print_help() {
    println!("minimize-git-diff-llm {}", env!("CARGO_PKG_VERSION"));
    println!("{}", env!("CARGO_PKG_DESCRIPTION"));
    println!();
    println!("USAGE:");
    println!("    git diff | minimize-git-diff-llm [OPTIONS]");
    println!();
    println!("OPTIONS:");
    println!("    -h, --help              Print help information");
    println!("    -V, --version           Print version information");
    println!("    --save                  Save diff chunks to separate files");
    println!("    --save-path <PATH>      Specify custom output directory (default: llm/diff)");
    println!();
    println!("EXAMPLES:");
    println!("    # Minimize diff from stdin");
    println!("    git diff | minimize-git-diff-llm");
    println!();
    println!("    # Save diff chunks to files");
    println!("    git diff | minimize-git-diff-llm --save");
    println!();
    println!("    # Save to custom directory");
    println!("    git diff | minimize-git-diff-llm --save --save-path custom/path");
    println!();
    println!("    # With staged changes");
    println!("    git diff --cached | minimize-git-diff-llm --save");
    println!();
    println!("OUTPUT (when using --save):");
    println!("    generated: <path>/<project-name>/");
    println!("    REVIEW.md: <absolute-path-to-REVIEW.md>");
    println!();
    println!(
        "For more information, visit: {}",
        env!("CARGO_PKG_REPOSITORY")
    );
}

fn parse_save_path(args: &[String]) -> Option<String> {
    for i in 0..args.len() {
        if args[i] == "--save-path" && i + 1 < args.len() {
            return Some(args[i + 1].clone());
        }
    }
    None
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
    // Get project identifier to create project-specific folder
    let project_id = get_project_identifier();
    let project_output_dir = format!("{}/{}", output_dir, project_id);

    // Remove the directory if it exists, then create it
    if Path::new(&project_output_dir).exists() {
        fs::remove_dir_all(&project_output_dir)?;
    }
    fs::create_dir_all(&project_output_dir)?;

    let file_changes = parse_git_diff(diff_content);

    // Prepare REVIEW.md content
    let mut review_content = String::from(
        "# Code Review Tracking\n\n\
        This file tracks the review status of code changes.\n\n\
        ## Guidelines\n\
        - Update `meta:status` after reviewing each file\n\
        - Status values: `pending`, `reviewed@YYYY-MM-DD`, `outdated`\n\
        - Add review comments in the placeholder section below each file\n\n\
        ---\n\n",
    );

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

        // Add entry to REVIEW.md
        review_content.push_str(&format!("## {}\n", filepath));
        review_content.push_str(&format!("- meta:hash: {}\n", file_hash));
        review_content.push_str(&format!("- meta:diff_chunk: {}\n", chunk_filename));
        review_content.push_str("- meta:status: pending\n\n");
        review_content.push_str("<!-- Review comments go here -->\n\n");
        review_content.push_str("---\n\n");
    }

    // Write REVIEW.md to current working directory
    let review_path = "REVIEW.md";
    let mut review_file = fs::File::create(review_path)?;
    review_file.write_all(review_content.as_bytes())?;

    // Get absolute path for REVIEW.md
    let review_absolute_path = env::current_dir()
        .ok()
        .and_then(|p| p.join(review_path).to_str().map(String::from))
        .unwrap_or_else(|| review_path.to_string());

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
        let _ = fs::remove_file("REVIEW.md");

        // Test save with default path
        save_diff_chunks(diff, "llm/diff").unwrap();

        // Get project identifier to verify project-specific folder
        let project_id = get_project_identifier();
        let project_dir = format!("llm/diff/{}", project_id);

        // Verify project directory exists
        assert!(Path::new(&project_dir).exists());

        // Verify chunk files exist in project-specific folder
        assert!(Path::new(&format!("{}/chunk_aa.diff", project_dir)).exists());
        assert!(Path::new(&format!("{}/chunk_ab.diff", project_dir)).exists());

        // Verify content
        let chunk_aa = fs::read_to_string(format!("{}/chunk_aa.diff", project_dir)).unwrap();
        assert!(chunk_aa.contains("diff --git a/file1.txt b/file1.txt"));
        assert!(chunk_aa.contains("+Line 1"));

        let chunk_ab = fs::read_to_string(format!("{}/chunk_ab.diff", project_dir)).unwrap();
        assert!(chunk_ab.contains("diff --git a/file2.txt b/file2.txt"));
        assert!(chunk_ab.contains("+Content"));

        // Verify REVIEW.md exists and has correct format
        assert!(Path::new("REVIEW.md").exists());
        let review = fs::read_to_string("REVIEW.md").unwrap();
        assert!(review.contains("# Code Review Tracking"));
        assert!(review.contains("## file1.txt"));
        assert!(review.contains("## file2.txt"));
        assert!(review.contains("meta:hash:"));
        assert!(review.contains("meta:diff_chunk: chunk_aa.diff"));
        assert!(review.contains("meta:diff_chunk: chunk_ab.diff"));
        assert!(review.contains("meta:status: pending"));

        // Clean up after test
        let _ = fs::remove_dir_all("llm/diff");
        let _ = fs::remove_file("REVIEW.md");
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
        let _ = fs::remove_file("REVIEW.md");

        // Test save with custom path
        save_diff_chunks(diff, custom_path).unwrap();

        // Get project identifier to verify project-specific folder
        let project_id = get_project_identifier();
        let project_dir = format!("{}/{}", custom_path, project_id);

        // Verify project directory exists
        assert!(Path::new(&project_dir).exists());

        // Verify chunk file exists
        assert!(Path::new(&format!("{}/chunk_aa.diff", project_dir)).exists());

        // Verify content
        let chunk_aa = fs::read_to_string(format!("{}/chunk_aa.diff", project_dir)).unwrap();
        assert!(chunk_aa.contains("diff --git a/test.txt b/test.txt"));
        assert!(chunk_aa.contains("+Test content"));

        // Verify REVIEW.md exists
        assert!(Path::new("REVIEW.md").exists());
        let review = fs::read_to_string("REVIEW.md").unwrap();
        assert!(review.contains("## test.txt"));
        assert!(review.contains("meta:diff_chunk: chunk_aa.diff"));

        // Clean up after test
        let _ = fs::remove_dir_all(custom_path);
        let _ = fs::remove_file("REVIEW.md");
    }

    #[test]
    fn test_parse_save_path() {
        // Test with --save-path argument
        let args = vec![
            "program".to_string(),
            "--save".to_string(),
            "--save-path".to_string(),
            "my/custom/path".to_string(),
        ];
        assert_eq!(parse_save_path(&args), Some("my/custom/path".to_string()));

        // Test without --save-path argument
        let args = vec!["program".to_string(), "--save".to_string()];
        assert_eq!(parse_save_path(&args), None);

        // Test with --save-path but no value
        let args = vec!["program".to_string(), "--save-path".to_string()];
        assert_eq!(parse_save_path(&args), None);
    }

    #[test]
    fn test_get_project_identifier() {
        // Should return a non-empty string
        let project_id = get_project_identifier();
        assert!(!project_id.is_empty());
        // In git repository, should return "minimize-git-diff-llm"
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
        let _ = fs::remove_file("REVIEW.md");

        // Save diff chunks
        save_diff_chunks(diff, "test_review").unwrap();

        // Verify REVIEW.md format
        assert!(Path::new("REVIEW.md").exists());
        let review = fs::read_to_string("REVIEW.md").unwrap();

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
        let _ = fs::remove_file("REVIEW.md");
    }
}
