use pinyin::ToPinyin;

/// Generate a slugified version of the input string suitable for branch names
/// - Converts Chinese characters to pinyin
/// - Converts to lowercase
/// - Replaces spaces and non-alphanumeric characters with hyphens
/// - Removes consecutive hyphens
/// - Trims hyphens from start and end
pub fn slugify(text: &str) -> String {
    let mut result = String::new();

    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() {
            result.push(ch.to_ascii_lowercase());
        } else if ch.is_ascii_whitespace() || ch == '-' || ch == '_' {
            if !result.is_empty() && !result.ends_with('-') {
                result.push('-');
            }
        } else if !ch.is_ascii() {
            // Convert Chinese characters to pinyin
            if let Some(pinyin) = ch.to_pinyin() {
                if !result.is_empty() && !result.ends_with('-') {
                    result.push('-');
                }
                result.push_str(pinyin.plain());
            }
        }
    }

    // Remove trailing hyphen
    if result.ends_with('-') {
        result.pop();
    }

    // Limit length
    if result.len() > 60 {
        result.truncate(60);
        // Remove trailing hyphen if truncation created one
        if result.ends_with('-') {
            result.pop();
        }
    }

    result
}

/// Generate a default branch name from description
pub fn generate_branch_name(desc: &str) -> String {
    slugify(desc)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slugify_ascii() {
        assert_eq!(slugify("Hello World"), "hello-world");
        assert_eq!(slugify("Test_Feature"), "test-feature");
        assert_eq!(slugify("Fix Bug #123"), "fix-bug-123");
    }

    #[test]
    fn test_slugify_chinese() {
        let result = slugify("实现登录功能");
        assert!(result.contains("shi"));
        assert!(result.contains("xian"));
        assert!(result.contains("deng"));
        assert!(result.contains("lu"));
        assert!(result.contains("gong"));
        assert!(result.contains("neng"));
    }

    #[test]
    fn test_slugify_mixed() {
        let result = slugify("Add 用户 Feature");
        assert!(result.starts_with("add"));
        assert!(result.contains("yong"));
        assert!(result.contains("hu"));
        assert!(result.contains("feature"));
    }

    #[test]
    fn test_slugify_special_chars() {
        assert_eq!(slugify("hello---world"), "hello-world");
        assert_eq!(slugify("   spaces   "), "spaces");
        assert_eq!(slugify("!!!test!!!"), "test");
    }

    #[test]
    fn test_slugify_length_limit() {
        let long_text = "this is a very long description that exceeds the maximum allowed length for branch names";
        let result = slugify(long_text);
        assert!(result.len() <= 60);
        assert!(!result.ends_with('-'));
    }

    #[test]
    fn test_generate_branch_name() {
        let name = generate_branch_name("Test Feature");
        assert_eq!(name, "test-feature");
    }
}
