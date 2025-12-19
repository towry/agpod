use crate::cli::{KiroArgs, KiroCommand};
use crate::config::Config;
use crate::error::KiroError;
use crate::git::GitHelper;
use crate::plugin::PluginExecutor;
use crate::template::{TemplateContext, TemplateRenderer};
use anyhow::Result;
use chrono::{DateTime, Local};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub fn execute(args: KiroArgs) -> Result<()> {
    // Check if it's an init command first
    let is_init = matches!(&args.command, Some(KiroCommand::Init { .. }));

    if is_init {
        if let Some(KiroCommand::Init { force }) = args.command {
            return cmd_init(force);
        }
    }

    // Check if config is initialized for other commands
    if !Config::is_initialized() {
        eprintln!("Error: agpod kiro is not initialized.");
        eprintln!();
        eprintln!("Please run the following command to initialize:");
        eprintln!("  agpod kiro init");
        eprintln!();
        if let Some(config_dir) = Config::get_config_dir() {
            eprintln!(
                "This will create configuration files in: {}",
                config_dir.display()
            );
        }
        std::process::exit(1);
    }

    // Load configuration
    let config = Config::load(args.config.as_deref(), &args)?;

    // Determine which command to run
    let command = if let Some(cmd) = args.command {
        cmd
    } else if let Some(desc) = args.pr_new {
        KiroCommand::PrNew {
            desc,
            template: None,
            force: false,
            git_branch: false,
            open: false,
        }
    } else if args.pr_list {
        KiroCommand::PrList {
            summary_lines: config.summary_lines,
            since: None,
            limit: None,
        }
    } else if args.pr {
        KiroCommand::Pr {
            fzf: false,
            output: "rel".to_string(),
        }
    } else {
        // No command specified, show help
        eprintln!("No command specified. Use --help for usage information.");
        std::process::exit(2);
    };

    match command {
        KiroCommand::PrNew {
            desc,
            template,
            force,
            git_branch,
            open,
        } => cmd_pr_new(
            &config,
            &desc,
            template.as_deref(),
            force,
            git_branch,
            open,
            args.dry_run,
        ),
        KiroCommand::PrList {
            summary_lines,
            since,
            limit,
        } => cmd_pr_list(&config, summary_lines, args.json, since, limit),
        KiroCommand::Pr { fzf, output } => cmd_pr(&config, fzf, &output),
        KiroCommand::ListTemplates => cmd_list_templates(&config, args.json),
        KiroCommand::Init { .. } => unreachable!(), // Already handled above
    }
}

fn cmd_pr_new(
    config: &Config,
    desc: &str,
    template: Option<&str>,
    force: bool,
    git_branch: bool,
    open: bool,
    dry_run: bool,
) -> Result<()> {
    let template_name = template.unwrap_or(&config.template);

    // Generate branch name
    let plugin_executor = PluginExecutor::new(config.clone());
    let branch_name = plugin_executor.generate_branch_name(desc, template_name)?;

    // Check if directory already exists
    let pr_dir = PathBuf::from(&config.base_dir).join(&branch_name);
    if pr_dir.exists() && !force {
        return Err(KiroError::DirectoryExists(pr_dir.display().to_string()).into());
    }

    if dry_run {
        println!("Dry run mode - would create:");
        println!("  Directory: {}", pr_dir.display());
        println!("  Branch name: {}", branch_name);
        return Ok(());
    }

    // Create directory
    fs::create_dir_all(&pr_dir)?;

    // Get git info
    let git_info = GitHelper::get_git_info();

    // Prepare template context
    let pr_dir_abs = fs::canonicalize(&pr_dir)?.to_string_lossy().to_string();
    let context = TemplateContext {
        name: branch_name.clone(),
        desc: desc.to_string(),
        template: template_name.to_string(),
        base_dir: config.base_dir.clone(),
        pr_dir_abs: pr_dir_abs.clone(),
        pr_dir_rel: pr_dir.to_string_lossy().to_string(),
        git_info,
    };

    // Render templates
    let mut renderer = TemplateRenderer::new(&config.templates_dir)?;

    // Get template-specific configuration
    let template_config = config.templates.get(template_name).ok_or_else(|| {
        KiroError::Config(format!(
            "Template '{}' not found in configuration. Please add a [kiro.templates.{}] section to your config file.",
            template_name, template_name
        ))
    })?;

    let rendered_files = renderer.render_all(
        template_name,
        &template_config.files,
        &context,
        config,
        &pr_dir,
        &template_config.missing_policy,
    )?;

    // Output results (machine-readable format)
    println!("{}", branch_name);

    // Log info to stderr
    eprintln!("Created: {}", pr_dir.display());
    for file in &rendered_files {
        eprintln!(
            "  Rendered: {}",
            file.file_name().unwrap().to_string_lossy()
        );
    }

    // Create git branch if requested
    if git_branch {
        match GitHelper::create_and_checkout_branch(&branch_name) {
            Ok(_) => eprintln!("Created and checked out branch: {}", branch_name),
            Err(e) => eprintln!("Warning: Failed to create git branch: {}", e),
        }
    }

    // Open in editor if requested
    if open {
        if let Some(first_file) = rendered_files.first() {
            open_in_editor(first_file)?;
        }
    }

    Ok(())
}

// Time conversion constants
const SECONDS_PER_MINUTE: u64 = 60;
const SECONDS_PER_HOUR: u64 = 3600;
const SECONDS_PER_DAY: u64 = 86400;
const SECONDS_PER_WEEK: u64 = 604800;
const SECONDS_PER_MONTH: u64 = 2592000; // Approximated as 30 days
const SECONDS_PER_YEAR: u64 = 31536000; // Approximated as 365 days

/// Parse a time expression like "2 days", "1 week", "3 hours" into a Duration
/// Note: Months are approximated as 30 days and years as 365 days
fn parse_time_expression(expr: &str) -> Result<std::time::Duration> {
    let expr = expr.trim().to_lowercase();
    let parts: Vec<&str> = expr.split_whitespace().collect();

    if parts.len() != 2 {
        return Err(anyhow::anyhow!(
            "Invalid time expression format. Expected '<number> <unit>' (e.g., '2 days', '1 week')"
        ));
    }

    let number: u64 = parts[0]
        .parse()
        .map_err(|_| anyhow::anyhow!("Invalid number in time expression"))?;

    let unit = parts[1];
    let multiplier = match unit {
        "second" | "seconds" | "sec" | "secs" | "s" => 1,
        "minute" | "minutes" | "min" | "mins" | "m" => SECONDS_PER_MINUTE,
        "hour" | "hours" | "hr" | "hrs" | "h" => SECONDS_PER_HOUR,
        "day" | "days" | "d" => SECONDS_PER_DAY,
        "week" | "weeks" | "w" => SECONDS_PER_WEEK,
        "month" | "months" => SECONDS_PER_MONTH,
        "year" | "years" | "y" => SECONDS_PER_YEAR,
        _ => {
            return Err(anyhow::anyhow!(
                "Unknown time unit '{}'. Supported units: seconds, minutes, hours, days, weeks, months (30 days), years (365 days)",
                unit
            ));
        }
    };

    // Check for overflow before multiplication
    let seconds = number
        .checked_mul(multiplier)
        .ok_or_else(|| anyhow::anyhow!("Time duration value too large"))?;

    Ok(std::time::Duration::from_secs(seconds))
}

fn cmd_pr_list(
    config: &Config,
    summary_lines: usize,
    json: bool,
    since: Option<String>,
    limit: Option<usize>,
) -> Result<()> {
    let base_dir = Path::new(&config.base_dir);

    if !base_dir.exists() {
        if json {
            println!("[]");
        }
        return Ok(());
    }

    let mut entries = Vec::new();

    // Scan directories
    for entry in fs::read_dir(base_dir)? {
        let entry = entry?;
        let path = entry.path();

        if !path.is_dir() {
            continue;
        }

        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();

        // Skip hidden directories
        if name.starts_with('.') {
            continue;
        }

        // Try to read DESIGN.md for summary
        let summary = read_summary(&path, summary_lines);

        // Get the modification time of DESIGN.md
        let mtime = get_design_mtime(&path);

        entries.push((name, summary, mtime));
    }

    // Sort by modification time (most recent first), fallback to name
    entries.sort_by(compare_by_mtime);

    // Filter by time range if --since is provided
    if let Some(since_expr) = since {
        let duration = parse_time_expression(&since_expr)?;
        let cutoff_time = SystemTime::now()
            .checked_sub(duration)
            .ok_or_else(|| anyhow::anyhow!("Time duration too large"))?;

        entries.retain(|(_, _, mtime)| {
            if let Some(time) = mtime {
                time >= &cutoff_time
            } else {
                false // Exclude entries without mtime when filtering by time
            }
        });
    }

    // Apply limit if specified
    if let Some(limit_count) = limit {
        entries.truncate(limit_count);
    }

    if json {
        let json_entries = create_json_entries(&entries, base_dir);
        println!("{}", serde_json::to_string_pretty(&json_entries)?);
    } else {
        // Table output
        println!("{:<40} SUMMARY", "NAME");
        println!("{}", "-".repeat(80));
        for (name, summary, mtime) in entries {
            // Format the display name with relative time if available
            let display_name = if let Some(time) = mtime {
                let formatted_name = format_name_for_display(&name);
                let relative_time = format_relative_time(time);
                format!("{} [{}]", formatted_name, relative_time)
            } else {
                format_name_for_display(&name)
            };

            let summary_display = if summary.is_empty() {
                "(no DESIGN.md)"
            } else {
                &summary
            };
            println!("{:<40} {}", display_name, summary_display);
        }
    }

    Ok(())
}

fn cmd_pr(config: &Config, _use_fzf: bool, output_format: &str) -> Result<()> {
    let base_dir = Path::new(&config.base_dir);

    if !base_dir.exists() {
        eprintln!(
            "Error: Base directory does not exist: {}",
            base_dir.display()
        );
        std::process::exit(1);
    }

    // Collect entries
    let mut entries = Vec::new();
    for entry in fs::read_dir(base_dir)? {
        let entry = entry?;
        let path = entry.path();

        if !path.is_dir() {
            continue;
        }

        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();

        if name.starts_with('.') {
            continue;
        }

        let summary = read_summary(&path, 1);

        // Get the modification time of DESIGN.md
        let mtime = get_design_mtime(&path);

        entries.push((name, summary, mtime));
    }

    if entries.is_empty() {
        eprintln!("No PR drafts found in {}", base_dir.display());
        return Ok(());
    }

    // Sort by modification time (most recent first), fallback to name
    entries.sort_by(compare_by_mtime);

    // Use fzf by default if available
    if is_fzf_available() {
        let selected = select_with_fzf(&entries, base_dir)?;
        output_result(&selected, output_format, base_dir);
    } else {
        let selected = select_with_dialoguer(&entries)?;
        output_result(&selected, output_format, base_dir);
    }

    Ok(())
}

fn read_summary(dir: &Path, max_lines: usize) -> String {
    let design_file = dir.join("DESIGN.md");
    if !design_file.exists() {
        return String::new();
    }

    match fs::read_to_string(&design_file) {
        Ok(content) => {
            let lines: Vec<&str> = content
                .lines()
                .filter(|line| !line.trim().is_empty())
                .take(max_lines)
                .collect();
            lines.join(" ")
        }
        Err(_) => String::new(),
    }
}

/// Get the modification time of DESIGN.md in a directory
fn get_design_mtime(dir: &Path) -> Option<std::time::SystemTime> {
    let design_path = dir.join("DESIGN.md");
    if design_path.exists() {
        fs::metadata(&design_path).and_then(|m| m.modified()).ok()
    } else {
        None
    }
}

/// Compare two entries by modification time (most recent first), then by name
fn compare_by_mtime(
    a: &(String, String, Option<std::time::SystemTime>),
    b: &(String, String, Option<std::time::SystemTime>),
) -> std::cmp::Ordering {
    match (&a.2, &b.2) {
        (Some(time_a), Some(time_b)) => time_b.cmp(time_a), // Most recent first
        (Some(_), None) => std::cmp::Ordering::Less,        // Files with mtime first
        (None, Some(_)) => std::cmp::Ordering::Greater,     // Files without mtime last
        (None, None) => a.0.cmp(&b.0),                      // Fallback to name
    }
}

/// Format a kebab-case name to Title Case with spaces
/// Example: "recently-modified-pr" -> "Recently modified pr"
fn format_name_for_display(name: &str) -> String {
    let words: Vec<&str> = name.split('-').collect();
    if words.is_empty() {
        return name.to_string();
    }

    // Capitalize first word, lowercase rest
    let mut result = String::new();
    for (i, word) in words.iter().enumerate() {
        if i == 0 {
            // Capitalize first letter of first word
            result.push_str(
                &word
                    .chars()
                    .next()
                    .map(|c| c.to_uppercase().to_string())
                    .unwrap_or_default(),
            );
            result.push_str(&word[1..].to_lowercase());
        } else {
            result.push(' ');
            result.push_str(&word.to_lowercase());
        }
    }
    result
}

/// Format a relative time string from a SystemTime
/// Example: "3min ago", "2 hours ago", "yesterday", "3 days ago"
fn format_relative_time(time: SystemTime) -> String {
    let datetime: DateTime<Local> = time.into();
    let now = Local::now();
    let duration = now.signed_duration_since(datetime);

    let seconds = duration.num_seconds();
    let minutes = duration.num_minutes();
    let hours = duration.num_hours();
    let days = duration.num_days();

    if seconds < 60 {
        "just now".to_string()
    } else if minutes < 60 {
        format!("{}min ago", minutes)
    } else if hours < 24 {
        if hours == 1 {
            "1 hour ago".to_string()
        } else {
            format!("{} hours ago", hours)
        }
    } else if days == 1 {
        "yesterday".to_string()
    } else if days < 7 {
        format!("{} days ago", days)
    } else if days < 30 {
        let weeks = days / 7;
        if weeks == 1 {
            "1 week ago".to_string()
        } else {
            format!("{} weeks ago", weeks)
        }
    } else if days < 365 {
        let months = days / 30;
        if months == 1 {
            "1 month ago".to_string()
        } else {
            format!("{} months ago", months)
        }
    } else {
        let years = days / 365;
        if years == 1 {
            "1 year ago".to_string()
        } else {
            format!("{} years ago", years)
        }
    }
}

/// Format a SystemTime to "YYYY-MM-DD HH:MM" format
/// Example: "2025-10-10 10:10"
fn format_datetime(time: SystemTime) -> String {
    let datetime: DateTime<Local> = time.into();
    datetime.format("%Y-%m-%d %H:%M").to_string()
}

/// Create JSON entries from PR list entries
fn create_json_entries(
    entries: &[(String, String, Option<SystemTime>)],
    base_dir: &Path,
) -> Vec<serde_json::Value> {
    entries
        .iter()
        .map(|(name, summary, mtime)| {
            let rel_path = base_dir.join(name);
            let mut entry = serde_json::json!({
                "name": name,
                "summary": summary,
                "path": rel_path.to_string_lossy()
            });

            // Add date field if modification time is available
            if let Some(time) = mtime {
                entry["date"] = serde_json::json!(format_datetime(*time));
            }

            entry
        })
        .collect()
}

fn is_fzf_available() -> bool {
    std::process::Command::new("fzf")
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn select_with_fzf(
    entries: &[(String, String, Option<std::time::SystemTime>)],
    base_dir: &Path,
) -> Result<String> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    // Build fzf command with fuzzy filter options
    let mut cmd = Command::new("fzf");
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .arg("--height=40%")
        .arg("--reverse")
        .arg("--prompt=Select PR: ")
        .arg("--border")
        .arg("--info=inline")
        .arg("--delimiter=\t")
        .arg("--with-nth=2"); // Show only field 2 (formatted name)

    // Add preview to show DESIGN.md content from the selected PR directory
    // {1} refers to the first field (original directory name)
    let preview_cmd = format!(
        "test -f {}/{{1}}/DESIGN.md && cat {}/{{1}}/DESIGN.md || echo 'No DESIGN.md found'",
        base_dir.display(),
        base_dir.display()
    );
    cmd.arg("--preview").arg(preview_cmd);
    cmd.arg("--preview-window=right:50%:wrap");

    let mut child = cmd.spawn()?;

    {
        let stdin = child.stdin.as_mut().unwrap();
        for (name, _summary, mtime) in entries {
            // Format: "original-name<TAB>Formatted name [timestamp]"
            // Field 1 (original name) is hidden but used for preview and selection
            // Field 2 (formatted name) is displayed
            let formatted_name = if let Some(time) = mtime {
                let display = format_name_for_display(name);
                let relative_time = format_relative_time(*time);
                format!("{} [{}]", display, relative_time)
            } else {
                format_name_for_display(name)
            };

            let line = format!("{}\t{}\n", name, formatted_name);
            stdin.write_all(line.as_bytes())?;
        }
    }

    let output = child.wait_with_output()?;

    if output.status.success() {
        let selected = String::from_utf8_lossy(&output.stdout);
        // fzf returns the full line, extract the first field (original name)
        let name = selected.split('\t').next().unwrap_or("").trim();
        Ok(name.to_string())
    } else {
        std::process::exit(1);
    }
}

fn select_with_dialoguer(
    entries: &[(String, String, Option<std::time::SystemTime>)],
) -> Result<String> {
    let items: Vec<String> = entries
        .iter()
        .map(|(name, _summary, mtime)| {
            // Format same as pr-list and fzf: "Formatted name [timestamp]"
            // No summary shown here to match fzf behavior
            if let Some(time) = mtime {
                let display = format_name_for_display(name);
                let relative_time = format_relative_time(*time);
                format!("{} [{}]", display, relative_time)
            } else {
                format_name_for_display(name)
            }
        })
        .collect();

    let selection = dialoguer::Select::new()
        .with_prompt("Select a PR draft")
        .items(&items)
        .default(0)
        .interact()?;

    Ok(entries[selection].0.clone())
}

fn output_result(name: &str, format: &str, base_dir: &Path) {
    match format {
        "name" => println!("{}", name),
        "abs" => {
            let abs_path = base_dir.join(name);
            if let Ok(canonical) = fs::canonicalize(&abs_path) {
                println!("{}", canonical.display());
            } else {
                println!("{}", abs_path.display());
            }
        }
        "rel" => {
            // Relative path from cwd: base_dir/name
            let rel_path = base_dir.join(name);
            println!("{}", rel_path.display());
        }
        _ => {
            // Default to name
            println!("{}", name);
        }
    }
}

fn open_in_editor(file: &Path) -> Result<()> {
    let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vi".to_string());

    std::process::Command::new(&editor).arg(file).status()?;

    Ok(())
}

fn cmd_list_templates(config: &Config, json: bool) -> Result<()> {
    let templates_dir = Path::new(&config.templates_dir);

    if !templates_dir.exists() {
        if json {
            println!("[]");
        } else {
            eprintln!("Templates directory not found: {}", templates_dir.display());
        }
        return Ok(());
    }

    let mut templates = Vec::new();

    // Scan template directories
    for entry in fs::read_dir(templates_dir)? {
        let entry = entry?;
        let path = entry.path();

        if !path.is_dir() {
            continue;
        }

        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();

        // Skip special directories (like _shared)
        if name.starts_with('_') || name.starts_with('.') {
            continue;
        }

        // Get description from config if available
        let description = if let Some(template_config) = config.templates.get(&name) {
            template_config.description.clone()
        } else {
            String::new()
        };

        templates.push((name, description));
    }

    // Sort by name
    templates.sort_by(|a, b| a.0.cmp(&b.0));

    if json {
        let json_templates: Vec<serde_json::Value> = templates
            .iter()
            .map(|(name, description)| {
                serde_json::json!({
                    "name": name,
                    "description": description
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&json_templates)?);
    } else {
        // Table output
        println!("{:<20} DESCRIPTION", "TEMPLATE");
        println!("{}", "-".repeat(80));
        for (name, description) in templates {
            let description_display = if description.is_empty() {
                "(no description)"
            } else {
                &description
            };
            println!("{:<20} {}", name, description_display);
        }
    }

    Ok(())
}

fn cmd_init(force: bool) -> Result<()> {
    let config_dir = Config::get_config_dir()
        .ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;

    // Check if already initialized
    if config_dir.exists() && !force {
        eprintln!(
            "Configuration directory already exists: {}",
            config_dir.display()
        );
        eprintln!("Use --force to re-initialize.");
        return Ok(());
    }

    // Create directory structure
    let templates_dir = config_dir.join("templates");
    let plugins_dir = config_dir.join("plugins");
    let shared_dir = templates_dir.join("_shared");
    let default_dir = templates_dir.join("default");

    fs::create_dir_all(&shared_dir)?;
    fs::create_dir_all(&default_dir)?;
    fs::create_dir_all(&plugins_dir)?;

    // Create config file
    let config_file = config_dir.join("config.toml");
    if force || !config_file.exists() {
        let config_content = include_str!("../../../examples/config.toml");
        fs::write(&config_file, config_content)?;
        eprintln!("Created config file: {}", config_file.display());
    }

    // Create base templates
    let base_design = shared_dir.join("base_design.md.j2");
    if force || !base_design.exists() {
        let content = include_str!("../../../examples/templates/_shared/base_design.md.j2");
        fs::write(&base_design, content)?;
        eprintln!("Created base template: {}", base_design.display());
    }

    let base_task = shared_dir.join("base_task.md.j2");
    if force || !base_task.exists() {
        let content = include_str!("../../../examples/templates/_shared/base_task.md.j2");
        fs::write(&base_task, content)?;
        eprintln!("Created base template: {}", base_task.display());
    }

    // Create default templates
    let default_design = default_dir.join("DESIGN.md.j2");
    if force || !default_design.exists() {
        let content = include_str!("../../../examples/templates/default/DESIGN.md.j2");
        fs::write(&default_design, content)?;
        eprintln!("Created template: {}", default_design.display());
    }

    let default_task = default_dir.join("TASK.md.j2");
    if force || !default_task.exists() {
        let content = include_str!("../../../examples/templates/default/TASK.md.j2");
        fs::write(&default_task, content)?;
        eprintln!("Created template: {}", default_task.display());
    }

    // Create example plugin
    let plugin_file = plugins_dir.join("name.sh");
    if force || !plugin_file.exists() {
        let content = include_str!("../../../examples/plugins/name.sh");
        fs::write(&plugin_file, content)?;

        // Make plugin executable on Unix
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&plugin_file)?.permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&plugin_file, perms)?;
        }

        eprintln!("Created plugin: {}", plugin_file.display());
    }

    eprintln!();
    eprintln!("✓ agpod kiro initialized successfully!");
    eprintln!();
    eprintln!("Configuration directory: {}", config_dir.display());
    eprintln!();
    eprintln!("You can now use:");
    eprintln!("  agpod kiro pr-new --desc \"your description\"");
    eprintln!();
    eprintln!("To add more templates, copy them to:");
    eprintln!("  {}", templates_dir.display());

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_read_summary() {
        let temp_dir = TempDir::new().unwrap();
        let design_file = temp_dir.path().join("DESIGN.md");
        fs::write(&design_file, "# Title\n\nLine 1\n\nLine 2\n\nLine 3").unwrap();

        let summary = read_summary(temp_dir.path(), 2);
        assert!(summary.contains("# Title"));
        assert!(summary.contains("Line 1"));
    }

    #[test]
    fn test_read_summary_missing_file() {
        let temp_dir = TempDir::new().unwrap();
        let summary = read_summary(temp_dir.path(), 3);
        assert_eq!(summary, "");
    }

    #[test]
    fn test_list_templates() {
        // Create a temporary config directory with templates
        let temp_dir = TempDir::new().unwrap();
        let templates_dir = temp_dir.path().join("templates");
        fs::create_dir_all(&templates_dir).unwrap();

        // Create some template directories
        fs::create_dir_all(templates_dir.join("default")).unwrap();
        fs::create_dir_all(templates_dir.join("rust")).unwrap();
        fs::create_dir_all(templates_dir.join("vue")).unwrap();
        fs::create_dir_all(templates_dir.join("_shared")).unwrap(); // Should be skipped

        // Create a config with template descriptions
        use crate::config::TemplateConfig;
        let mut templates = std::collections::HashMap::new();
        templates.insert(
            "rust".to_string(),
            TemplateConfig {
                description: "Rust template".to_string(),
                files: vec![],
                missing_policy: "error".to_string(),
            },
        );

        let config = Config {
            templates_dir: templates_dir.to_string_lossy().to_string(),
            templates,
            ..Default::default()
        };

        // Test that cmd_list_templates can run without errors
        let result = cmd_list_templates(&config, false);
        assert!(result.is_ok());

        // Test JSON output
        let result_json = cmd_list_templates(&config, true);
        assert!(result_json.is_ok());
    }

    #[test]
    fn test_pr_list_json_includes_path() {
        use tempfile::TempDir;

        // Create a temporary base directory with PR drafts
        let temp_dir = TempDir::new().unwrap();
        let base_dir = temp_dir.path().join("llm").join("kiro");
        fs::create_dir_all(&base_dir).unwrap();

        // Create test PR directories
        let pr1_dir = base_dir.join("test-pr-1");
        let pr2_dir = base_dir.join("test-pr-2");
        fs::create_dir_all(&pr1_dir).unwrap();
        fs::create_dir_all(&pr2_dir).unwrap();

        // Create DESIGN.md files
        let design1 = pr1_dir.join("DESIGN.md");
        let design2 = pr2_dir.join("DESIGN.md");
        fs::write(&design1, "# PR 1\n\nTest description").unwrap();
        // Sleep briefly to ensure different mtimes
        std::thread::sleep(std::time::Duration::from_millis(10));
        fs::write(&design2, "# PR 2\n\nAnother test").unwrap();

        // Create a config with the temp base_dir
        let config = Config {
            base_dir: base_dir.to_string_lossy().to_string(),
            ..Default::default()
        };

        // We can't easily capture stdout in a test, so we'll test the logic directly
        // by inspecting what would be generated
        let mut entries = Vec::new();
        for entry in fs::read_dir(&base_dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();
            if name.starts_with('.') {
                continue;
            }
            let summary = read_summary(&path, 3);

            // Get the modification time of DESIGN.md
            let mtime = get_design_mtime(&path);

            entries.push((name, summary, mtime));
        }

        // Sort by modification time (most recent first), fallback to name
        entries.sort_by(compare_by_mtime);

        let json_entries: Vec<serde_json::Value> = entries
            .iter()
            .map(|(name, summary, _)| {
                let rel_path = Path::new(&config.base_dir).join(name);
                serde_json::json!({
                    "name": name,
                    "summary": summary,
                    "path": rel_path.to_string_lossy()
                })
            })
            .collect();

        // Verify that entries include the path field
        assert_eq!(json_entries.len(), 2);

        // The first entry should be test-pr-2 (most recently modified)
        let first_entry = &json_entries[0];
        assert_eq!(first_entry["name"].as_str().unwrap(), "test-pr-2");
        assert!(first_entry["path"]
            .as_str()
            .unwrap()
            .ends_with("llm/kiro/test-pr-2"));
        assert!(first_entry["summary"].as_str().unwrap().contains("PR 2"));

        // The second entry should be test-pr-1
        let second_entry = &json_entries[1];
        assert_eq!(second_entry["name"].as_str().unwrap(), "test-pr-1");
        assert!(second_entry["path"]
            .as_str()
            .unwrap()
            .ends_with("llm/kiro/test-pr-1"));
        assert!(second_entry["summary"].as_str().unwrap().contains("PR 1"));
    }

    #[test]
    fn test_pr_list_sorts_by_mtime() {
        use tempfile::TempDir;

        // Create a temporary base directory with PR drafts
        let temp_dir = TempDir::new().unwrap();
        let base_dir = temp_dir.path().join("llm").join("kiro");
        fs::create_dir_all(&base_dir).unwrap();

        // Create test PR directories in specific order
        let pr_old = base_dir.join("old-pr");
        let pr_new = base_dir.join("new-pr");
        let pr_middle = base_dir.join("middle-pr");
        let pr_no_design = base_dir.join("no-design-pr");

        fs::create_dir_all(&pr_old).unwrap();
        fs::create_dir_all(&pr_new).unwrap();
        fs::create_dir_all(&pr_middle).unwrap();
        fs::create_dir_all(&pr_no_design).unwrap();

        // Create DESIGN.md files with different modification times
        // Old PR - created first
        fs::write(pr_old.join("DESIGN.md"), "# Old PR").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));

        // Middle PR - created second
        fs::write(pr_middle.join("DESIGN.md"), "# Middle PR").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));

        // New PR - created last (most recent)
        fs::write(pr_new.join("DESIGN.md"), "# New PR").unwrap();

        // No DESIGN.md for pr_no_design - should be at the end

        // Test the logic directly
        let mut entries = Vec::new();
        for entry in fs::read_dir(&base_dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();
            if name.starts_with('.') {
                continue;
            }
            let summary = read_summary(&path, 3);

            let mtime = get_design_mtime(&path);

            entries.push((name, summary, mtime));
        }

        // Sort by modification time (most recent first)
        entries.sort_by(compare_by_mtime);

        // Verify the order: new-pr, middle-pr, old-pr, no-design-pr
        assert_eq!(entries.len(), 4);
        assert_eq!(entries[0].0, "new-pr");
        assert_eq!(entries[1].0, "middle-pr");
        assert_eq!(entries[2].0, "old-pr");
        assert_eq!(entries[3].0, "no-design-pr");

        // Verify that entries with DESIGN.md come before those without
        assert!(entries[0].2.is_some());
        assert!(entries[1].2.is_some());
        assert!(entries[2].2.is_some());
        assert!(entries[3].2.is_none());
    }

    #[test]
    fn test_format_name_for_display() {
        assert_eq!(
            format_name_for_display("recently-modified-pr"),
            "Recently modified pr"
        );
        assert_eq!(format_name_for_display("pr-5-days-ago"), "Pr 5 days ago");
        assert_eq!(format_name_for_display("single"), "Single");
        assert_eq!(
            format_name_for_display("multiple-word-name-here"),
            "Multiple word name here"
        );
    }

    #[test]
    fn test_format_relative_time() {
        use std::time::Duration;

        let now = SystemTime::now();

        // Just now
        let time = now - Duration::from_secs(30);
        assert_eq!(format_relative_time(time), "just now");

        // Minutes ago
        let time = now - Duration::from_secs(3 * 60);
        assert_eq!(format_relative_time(time), "3min ago");

        let time = now - Duration::from_secs(30 * 60);
        assert_eq!(format_relative_time(time), "30min ago");

        // Hours ago
        let time = now - Duration::from_secs(2 * 3600);
        assert_eq!(format_relative_time(time), "2 hours ago");

        let time = now - Duration::from_secs(3600);
        assert_eq!(format_relative_time(time), "1 hour ago");

        // Days ago
        let time = now - Duration::from_secs(25 * 3600);
        assert_eq!(format_relative_time(time), "yesterday");

        let time = now - Duration::from_secs(5 * 24 * 3600);
        assert_eq!(format_relative_time(time), "5 days ago");
    }

    #[test]
    fn test_format_datetime() {
        use std::time::Duration;

        let now = SystemTime::now();
        let formatted = format_datetime(now);

        // Verify format matches "YYYY-MM-DD HH:MM" pattern
        assert!(formatted.len() == 16, "Expected format: YYYY-MM-DD HH:MM");
        assert_eq!(&formatted[4..5], "-");
        assert_eq!(&formatted[7..8], "-");
        assert_eq!(&formatted[10..11], " ");
        assert_eq!(&formatted[13..14], ":");

        // Test with a specific known time
        let specific_time = now - Duration::from_secs(3600); // 1 hour ago
        let formatted_specific = format_datetime(specific_time);
        assert!(formatted_specific.len() == 16);
    }

    #[test]
    fn test_pr_list_json_includes_date() {
        use tempfile::TempDir;

        // Create a temporary base directory with PR drafts
        let temp_dir = TempDir::new().unwrap();
        let base_dir = temp_dir.path().join("llm").join("kiro");
        fs::create_dir_all(&base_dir).unwrap();

        // Create test PR directories
        let pr1_dir = base_dir.join("test-pr-with-date");
        let pr2_dir = base_dir.join("test-pr-no-design");
        fs::create_dir_all(&pr1_dir).unwrap();
        fs::create_dir_all(&pr2_dir).unwrap();

        // Create DESIGN.md for first PR only
        let design1 = pr1_dir.join("DESIGN.md");
        fs::write(&design1, "# PR with date\n\nTest description").unwrap();

        // Test the logic directly by replicating cmd_pr_list JSON generation
        let mut entries = Vec::new();
        for entry in fs::read_dir(&base_dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();
            if name.starts_with('.') {
                continue;
            }
            let summary = read_summary(&path, 3);
            let mtime = get_design_mtime(&path);
            entries.push((name, summary, mtime));
        }

        entries.sort_by(compare_by_mtime);

        let json_entries = create_json_entries(&entries, &base_dir);

        // Verify entries structure
        assert_eq!(json_entries.len(), 2);

        // Find the entry with DESIGN.md
        let pr_with_date = json_entries
            .iter()
            .find(|e| e["name"].as_str().unwrap() == "test-pr-with-date")
            .unwrap();

        // Verify it has the date field
        assert!(
            pr_with_date.get("date").is_some(),
            "Date field should be present"
        );
        let date_str = pr_with_date["date"].as_str().unwrap();
        assert_eq!(date_str.len(), 16, "Date format should be YYYY-MM-DD HH:MM");
        assert_eq!(&date_str[4..5], "-");
        assert_eq!(&date_str[7..8], "-");
        assert_eq!(&date_str[10..11], " ");
        assert_eq!(&date_str[13..14], ":");

        // Find the entry without DESIGN.md
        let pr_no_design = json_entries
            .iter()
            .find(|e| e["name"].as_str().unwrap() == "test-pr-no-design")
            .unwrap();

        // Verify it does NOT have the date field
        assert!(
            pr_no_design.get("date").is_none(),
            "Date field should be absent when no DESIGN.md"
        );
    }

    #[test]
    fn test_parse_time_expression() {
        // Test various time expressions
        assert_eq!(
            parse_time_expression("2 days").unwrap(),
            std::time::Duration::from_secs(2 * 86400)
        );
        assert_eq!(
            parse_time_expression("1 week").unwrap(),
            std::time::Duration::from_secs(604800)
        );
        assert_eq!(
            parse_time_expression("3 hours").unwrap(),
            std::time::Duration::from_secs(3 * 3600)
        );
        assert_eq!(
            parse_time_expression("5 minutes").unwrap(),
            std::time::Duration::from_secs(5 * 60)
        );
        assert_eq!(
            parse_time_expression("30 seconds").unwrap(),
            std::time::Duration::from_secs(30)
        );

        // Test abbreviations
        assert_eq!(
            parse_time_expression("2 d").unwrap(),
            std::time::Duration::from_secs(2 * 86400)
        );
        assert_eq!(
            parse_time_expression("1 w").unwrap(),
            std::time::Duration::from_secs(604800)
        );
        assert_eq!(
            parse_time_expression("3 h").unwrap(),
            std::time::Duration::from_secs(3 * 3600)
        );

        // Test case insensitivity
        assert_eq!(
            parse_time_expression("2 DAYS").unwrap(),
            std::time::Duration::from_secs(2 * 86400)
        );
        assert_eq!(
            parse_time_expression("1 Week").unwrap(),
            std::time::Duration::from_secs(604800)
        );

        // Test invalid expressions
        assert!(parse_time_expression("invalid").is_err());
        assert!(parse_time_expression("2").is_err());
        assert!(parse_time_expression("days").is_err());
        assert!(parse_time_expression("2 invalid_unit").is_err());

        // Test overflow protection
        let result = parse_time_expression("999999999999999 years");
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Time duration value too large"));
    }

    #[test]
    fn test_pr_list_with_since_filter() {
        use tempfile::TempDir;

        // Create a temporary base directory with PR drafts
        let temp_dir = TempDir::new().unwrap();
        let base_dir = temp_dir.path().join("llm").join("kiro");
        fs::create_dir_all(&base_dir).unwrap();

        // Create PR directories with different modification times
        let pr_recent = base_dir.join("recent-pr");
        let pr_old = base_dir.join("old-pr");
        fs::create_dir_all(&pr_recent).unwrap();
        fs::create_dir_all(&pr_old).unwrap();

        // Create DESIGN.md files
        // Recent PR (created now)
        fs::write(pr_recent.join("DESIGN.md"), "# Recent PR").unwrap();

        // Old PR (we'll manually set an old mtime by creating it and sleeping)
        // Since we can't easily manipulate file mtimes in tests, we'll just verify the filtering logic
        // by checking that entries are filtered correctly based on mtime

        // Get entries
        let mut entries = Vec::new();
        for entry in fs::read_dir(&base_dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();
            if name.starts_with('.') {
                continue;
            }
            let summary = read_summary(&path, 3);
            let mtime = get_design_mtime(&path);
            entries.push((name, summary, mtime));
        }

        entries.sort_by(compare_by_mtime);

        // Test that recent entries are within the last day
        let duration = parse_time_expression("1 day").unwrap();
        let cutoff_time = SystemTime::now().checked_sub(duration).unwrap();

        let filtered: Vec<_> = entries
            .iter()
            .filter(|(_, _, mtime)| {
                if let Some(time) = mtime {
                    time >= &cutoff_time
                } else {
                    false
                }
            })
            .collect();

        // The recent-pr should be included
        assert!(!filtered.is_empty());
        assert!(filtered.iter().any(|(name, _, _)| name == "recent-pr"));
    }

    #[test]
    fn test_pr_list_with_limit() {
        use tempfile::TempDir;

        // Create a temporary base directory with multiple PR drafts
        let temp_dir = TempDir::new().unwrap();
        let base_dir = temp_dir.path().join("llm").join("kiro");
        fs::create_dir_all(&base_dir).unwrap();

        // Create multiple PR directories
        for i in 1..=5 {
            let pr_dir = base_dir.join(format!("pr-{}", i));
            fs::create_dir_all(&pr_dir).unwrap();
            fs::write(pr_dir.join("DESIGN.md"), format!("# PR {}", i)).unwrap();
            // Sleep briefly to ensure different mtimes
            if i < 5 {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }

        // Get entries
        let mut entries = Vec::new();
        for entry in fs::read_dir(&base_dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();
            if name.starts_with('.') {
                continue;
            }
            let summary = read_summary(&path, 3);
            let mtime = get_design_mtime(&path);
            entries.push((name, summary, mtime));
        }

        entries.sort_by(compare_by_mtime);

        // Test limit
        let limit_count = 2;
        entries.truncate(limit_count);

        assert_eq!(entries.len(), 2);
    }
}
