use std::fs;
use std::path::{Path, PathBuf};
use anyhow::Result;
use crate::kilo::cli::{KiloArgs, KiloCommand};
use crate::kilo::config::Config;
use crate::kilo::error::KiloError;
use crate::kilo::git::GitHelper;
use crate::kilo::plugin::PluginExecutor;
use crate::kilo::template::{TemplateContext, TemplateRenderer};


pub fn execute(args: KiloArgs) -> Result<()> {
    // Load configuration
    let config = Config::load(args.config.as_deref(), &args)?;
    
    // Determine which command to run
    let command = if let Some(cmd) = args.command {
        cmd
    } else if let Some(desc) = args.pr_new {
        KiloCommand::PrNew {
            desc,
            template: None,
            force: false,
            git_branch: false,
            open: false,
        }
    } else if args.pr_list {
        KiloCommand::PrList {
            summary_lines: config.summary_lines,
        }
    } else if args.pr {
        KiloCommand::Pr {
            fzf: false,
            output: "rel".to_string(),
        }
    } else {
        // No command specified, show help
        eprintln!("No command specified. Use --help for usage information.");
        std::process::exit(2);
    };
    
    match command {
        KiloCommand::PrNew {
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
        KiloCommand::PrList { summary_lines } => {
            cmd_pr_list(&config, summary_lines, args.json)
        }
        KiloCommand::Pr { fzf, output } => cmd_pr(&config, fzf, &output),
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
    let branch_name = plugin_executor.generate_branch_name(desc, template_name, "feature-impl")?;
    
    // Check if directory already exists
    let pr_dir = PathBuf::from(&config.base_dir).join(&branch_name);
    if pr_dir.exists() && !force {
        return Err(KiloError::DirectoryExists(pr_dir.display().to_string()).into());
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
    let pr_dir_abs = fs::canonicalize(&pr_dir)?
        .to_string_lossy()
        .to_string();
    let context = TemplateContext {
        branch_name: branch_name.clone(),
        desc: desc.to_string(),
        template: template_name.to_string(),
        base_dir: config.base_dir.clone(),
        pr_dir_abs: pr_dir_abs.clone(),
        pr_dir_rel: branch_name.clone(),
        git_info,
    };
    
    // Render templates
    let mut renderer = TemplateRenderer::new(&config.templates_dir)?;
    
    // Get files to render
    let files = if let Some(template_config) = config.templates.get(template_name) {
        template_config.files.clone()
    } else {
        config.rendering.files.clone()
    };
    
    let missing_policy = if let Some(template_config) = config.templates.get(template_name) {
        template_config.missing_policy.clone()
    } else {
        config.rendering.missing_policy.clone()
    };
    
    let rendered_files = renderer.render_all(
        template_name,
        &files,
        &context,
        config,
        &pr_dir,
        &missing_policy,
    )?;
    
    // Output results (machine-readable format)
    println!("{}", branch_name);
    
    // Log info to stderr
    eprintln!("Created: {}", pr_dir.display());
    for file in &rendered_files {
        eprintln!("  Rendered: {}", file.file_name().unwrap().to_string_lossy());
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

fn cmd_pr_list(config: &Config, summary_lines: usize, json: bool) -> Result<()> {
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
        
        let name = path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();
        
        // Skip hidden directories
        if name.starts_with('.') {
            continue;
        }
        
        // Try to read DESIGN.md for summary
        let summary = read_summary(&path, summary_lines);
        
        entries.push((name, summary));
    }
    
    // Sort by name
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    
    if json {
        let json_entries: Vec<serde_json::Value> = entries
            .iter()
            .map(|(name, summary)| {
                serde_json::json!({
                    "name": name,
                    "summary": summary
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&json_entries)?);
    } else {
        // Table output
        println!("{:<40} {}", "NAME", "SUMMARY");
        println!("{}", "-".repeat(80));
        for (name, summary) in entries {
            let summary_display = if summary.is_empty() {
                "(no DESIGN.md)"
            } else {
                &summary
            };
            println!("{:<40} {}", name, summary_display);
        }
    }
    
    Ok(())
}

fn cmd_pr(config: &Config, use_fzf: bool, output_format: &str) -> Result<()> {
    let base_dir = Path::new(&config.base_dir);
    
    if !base_dir.exists() {
        eprintln!("Error: Base directory does not exist: {}", base_dir.display());
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
        
        let name = path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();
        
        if name.starts_with('.') {
            continue;
        }
        
        let summary = read_summary(&path, 1);
        entries.push((name, summary));
    }
    
    if entries.is_empty() {
        eprintln!("No PR drafts found in {}", base_dir.display());
        return Ok(());
    }
    
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    
    // Try to use fzf if requested and available
    if use_fzf && is_fzf_available() {
        let selected = select_with_fzf(&entries)?;
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
            let lines: Vec<&str> = content.lines()
                .filter(|line| !line.trim().is_empty())
                .take(max_lines)
                .collect();
            lines.join(" ")
        }
        Err(_) => String::new(),
    }
}

fn is_fzf_available() -> bool {
    std::process::Command::new("fzf")
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn select_with_fzf(entries: &[(String, String)]) -> Result<String> {
    use std::process::{Command, Stdio};
    use std::io::Write;
    
    let mut child = Command::new("fzf")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;
    
    {
        let stdin = child.stdin.as_mut().unwrap();
        for (name, summary) in entries {
            let line = if summary.is_empty() {
                format!("{}\n", name)
            } else {
                format!("{} - {}\n", name, summary)
            };
            stdin.write_all(line.as_bytes())?;
        }
    }
    
    let output = child.wait_with_output()?;
    
    if output.status.success() {
        let selected = String::from_utf8_lossy(&output.stdout);
        let name = selected.split_whitespace().next().unwrap_or("").trim();
        Ok(name.to_string())
    } else {
        std::process::exit(1);
    }
}

fn select_with_dialoguer(entries: &[(String, String)]) -> Result<String> {
    let items: Vec<String> = entries
        .iter()
        .map(|(name, summary)| {
            if summary.is_empty() {
                name.clone()
            } else {
                format!("{} - {}", name, summary)
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
        "rel" | _ => {
            // Relative to base_dir
            println!("{}", name);
        }
    }
}

fn open_in_editor(file: &Path) -> Result<()> {
    let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vi".to_string());
    
    std::process::Command::new(&editor)
        .arg(file)
        .status()?;
    
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
}
