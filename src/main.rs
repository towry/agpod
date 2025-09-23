use regex::Regex;
use std::collections::HashSet;
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
        if file_change.is_large {
            // Strategy 1: For large files, only show metadata
            result.push_str(&format_large_file_summary(&file_change));
        } else {
            // For smaller files, show the diff but remove excessive empty lines
            result.push_str(&format_regular_file_diff(&file_change));
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
    
    // Extract keywords for readable files
    if let Some(keywords) = extract_keywords(path, &file_change.content_lines) {
        summary.push_str(&format!("Keywords: {}\n", keywords.join(", ")));
    }
    
    summary
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

fn extract_keywords(file_path: &str, content_lines: &[String]) -> Option<Vec<String>> {
    let mut keywords = HashSet::new();
    
    // Only extract keywords from text-based files
    if !is_text_file(file_path) {
        return None;
    }
    
    // For JSON files, extract top-level keys
    if file_path.ends_with(".json") {
        extract_json_keywords(&mut keywords, content_lines);
    }
    
    // For other text files, extract common programming keywords
    if file_path.ends_with(".rs") || file_path.ends_with(".py") || 
       file_path.ends_with(".js") || file_path.ends_with(".ts") ||
       file_path.ends_with(".java") || file_path.ends_with(".cpp") {
        extract_code_keywords(&mut keywords, content_lines);
    }
    
    if keywords.is_empty() {
        None
    } else {
        let mut sorted_keywords: Vec<String> = keywords.into_iter().collect();
        sorted_keywords.sort();
        sorted_keywords.truncate(10); // Limit to 10 keywords
        Some(sorted_keywords)
    }
}

fn is_text_file(file_path: &str) -> bool {
    let text_extensions = [
        ".txt", ".md", ".json", ".yaml", ".yml", ".toml", ".xml", ".html", ".css",
        ".js", ".ts", ".py", ".rs", ".java", ".cpp", ".c", ".h", ".hpp", ".go",
        ".php", ".rb", ".sh", ".sql", ".csv", ".log"
    ];
    
    text_extensions.iter().any(|ext| file_path.ends_with(ext))
}

fn extract_json_keywords(keywords: &mut HashSet<String>, content_lines: &[String]) {
    let key_pattern = Regex::new(r#""([^"]+)"\s*:"#).unwrap();
    
    for line in content_lines {
        if line.starts_with('+') || line.starts_with('-') {
            let content = &line[1..]; // Remove +/- prefix
            for capture in key_pattern.captures_iter(content) {
                if let Some(key) = capture.get(1) {
                    keywords.insert(key.as_str().to_string());
                }
            }
        }
    }
}

fn extract_code_keywords(keywords: &mut HashSet<String>, content_lines: &[String]) {
    // Common programming patterns
    let patterns = [
        r"\bfn\s+(\w+)", // Rust functions
        r"\bclass\s+(\w+)", // Classes
        r"\bstruct\s+(\w+)", // Structs
        r"\binterface\s+(\w+)", // Interfaces
        r"\bdef\s+(\w+)", // Python functions
        r"\bfunction\s+(\w+)", // JavaScript functions
    ];
    
    let combined_pattern = patterns.join("|");
    if let Ok(re) = Regex::new(&combined_pattern) {
        for line in content_lines {
            if line.starts_with('+') || line.starts_with('-') {
                let content = &line[1..]; // Remove +/- prefix
                for capture in re.captures_iter(content) {
                    for i in 1..capture.len() {
                        if let Some(keyword) = capture.get(i) {
                            keywords.insert(keyword.as_str().to_string());
                        }
                    }
                }
            }
        }
    }
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
