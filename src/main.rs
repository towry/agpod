use regex::Regex;
use std::io::{self, Read};

fn main() {
    match process_git_diff() {
        Ok(()) => {}
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    }
}

fn process_git_diff() -> io::Result<()> {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;
    
    let minimized_diff = minimize_diff(&input);
    print!("{}", minimized_diff);
    
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
            if (line.starts_with('+') && !line.starts_with("+++")) ||
               (line.starts_with('-') && !line.starts_with("---")) {
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

fn format_large_file_summary(file_change: &FileChange) -> String {
    let unknown_path = "unknown".to_string();
    let path = file_change.new_path.as_ref()
        .or(file_change.old_path.as_ref())
        .unwrap_or(&unknown_path);
    
    let mut summary = format!("Large file change: {}\n", path);
    summary.push_str(&format!("Change type: {}\n", file_change.change_type.as_str()));
    summary.push_str(&format!("Content lines: {}\n", file_change.content_lines.len()));
    
    summary
}

fn format_deleted_file_summary(file_change: &FileChange) -> String {
    let unknown_path = "unknown".to_string();
    let path = file_change.old_path.as_ref()
        .or(file_change.new_path.as_ref())
        .unwrap_or(&unknown_path);
    
    format!("Deleted file: {}\n", path)
}

fn format_regular_file_diff(file_change: &FileChange) -> String {
    let unknown_path = "unknown".to_string();
    let path = file_change.new_path.as_ref()
        .or(file_change.old_path.as_ref())
        .unwrap_or(&unknown_path);
    
    let mut result = format!("diff --git a/{} b/{}\n", 
        file_change.old_path.as_ref().unwrap_or(path), 
        file_change.new_path.as_ref().unwrap_or(path));
    
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
        let mut diff = String::from(r#"diff --git a/large.txt b/large.txt
new file mode 100644
index 0000000..1234567
--- /dev/null
+++ b/large.txt
"#);
        
        // Add many lines to make it a "large" file (>100 changes)
        for i in 1..=110 {
            diff.push_str(&format!("@@ -{},0 +{},1 @@\n", i, i));
            diff.push_str(&format!("+Line {}\n", i));
        }
        
        let result = minimize_diff(&diff);
        assert!(result.contains("Large file change: large.txt"));
        assert!(result.contains("Change type: added"));
        assert!(!result.contains("+Line 50")); // Content should not be shown
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
}
