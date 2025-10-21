use crate::config::Config;
use crate::error::{KiroError, KiroResult};
use chrono::{Local, Utc};
use minijinja::{context, Environment, Value};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

pub struct TemplateContext {
    pub name: String,
    pub desc: String,
    pub template: String,
    pub base_dir: String,
    pub pr_dir_abs: String,
    pub pr_dir_rel: String,
    pub git_info: Option<GitInfo>,
}

pub struct GitInfo {
    pub repo_root: String,
    pub current_branch: Option<String>,
    pub short_sha: Option<String>,
}

pub struct TemplateRenderer {
    env: Environment<'static>,
    templates_dir: PathBuf,
}

impl TemplateRenderer {
    pub fn new(templates_dir: &str) -> KiroResult<Self> {
        let templates_path = PathBuf::from(templates_dir);
        if !templates_path.exists() {
            return Err(KiroError::Template(format!(
                "Templates directory not found: {}",
                templates_dir
            )));
        }

        let mut env = Environment::new();

        // Set up loader to support template inheritance ({% extends %})
        // This allows templates to extend base templates
        env.set_loader(minijinja::path_loader(&templates_path));

        // Add custom filters
        env.add_filter("slugify", |value: String| -> String {
            crate::slug::slugify(&value)
        });

        env.add_filter("truncate", |value: String, n: usize| -> String {
            if value.len() > n {
                format!("{}...", &value[..n.saturating_sub(3)])
            } else {
                value
            }
        });

        Ok(Self {
            env,
            templates_dir: templates_path,
        })
    }

    pub fn render_template(
        &mut self,
        template_name: &str,
        template_file: &str,
        context: &TemplateContext,
        config: &Config,
    ) -> KiroResult<String> {
        // With path loader, template path is relative to templates_dir
        // Format: {template_name}/{template_file}
        let template_path_in_loader = format!("{}/{}", template_name, template_file);
        let full_template_path = self.templates_dir.join(template_name).join(template_file);

        if !full_template_path.exists() {
            return Err(KiroError::TemplateNotFound(format!(
                "{} in {}",
                template_file,
                full_template_path.display()
            )));
        }

        // Create template context
        let now = Utc::now();
        let date = Local::now();
        let user = std::env::var("USER").unwrap_or_else(|_| "unknown".to_string());

        let mut git_ctx = HashMap::new();
        if let Some(ref git_info) = context.git_info {
            git_ctx.insert("repo_root", Value::from(git_info.repo_root.clone()));
            if let Some(ref branch) = git_info.current_branch {
                git_ctx.insert("current_branch", Value::from(branch.clone()));
            }
            if let Some(ref sha) = git_info.short_sha {
                git_ctx.insert("short_sha", Value::from(sha.clone()));
            }
        }

        let ctx = context! {
            name => &context.name,
            desc => &context.desc,
            template => &context.template,
            now => now.to_rfc3339(),
            date => date.format("%Y-%m-%d").to_string(),
            user => &user,
            base_dir => &context.base_dir,
            pr_dir_abs => &context.pr_dir_abs,
            pr_dir_rel => &context.pr_dir_rel,
            git => git_ctx,
            config => match serde_json::to_value(config) {
                Ok(v) => Value::from_serialize(&v),
                Err(_) => Value::from(()),
            },
        };

        // Use the path loader to get the template (supports {% extends %})
        let tmpl = self
            .env
            .get_template(&template_path_in_loader)
            .map_err(|e| KiroError::Template(format!("Failed to get template: {}", e)))?;

        tmpl.render(ctx)
            .map_err(|e| KiroError::Template(format!("Failed to render template: {}", e)))
    }

    pub fn render_all(
        &mut self,
        template_name: &str,
        files: &[String],
        context: &TemplateContext,
        config: &Config,
        output_dir: &Path,
        missing_policy: &str,
    ) -> KiroResult<Vec<PathBuf>> {
        let mut rendered_files = Vec::new();

        for file in files {
            match self.render_template(template_name, file, context, config) {
                Ok(content) => {
                    // Remove .j2 extension if present
                    let output_filename = if file.ends_with(".j2") {
                        &file[..file.len() - 3]
                    } else {
                        file
                    };

                    let output_path = output_dir.join(output_filename);

                    fs::write(&output_path, content).map_err(KiroError::Io)?;

                    rendered_files.push(output_path);
                }
                Err(e) => {
                    if missing_policy == "error" {
                        return Err(e);
                    } else {
                        eprintln!("Warning: {}", e);
                    }
                }
            }
        }

        Ok(rendered_files)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn create_test_template(dir: &Path, template_name: &str, content: &str) -> PathBuf {
        let template_dir = dir.join(template_name);
        fs::create_dir_all(&template_dir).unwrap();
        let template_file = template_dir.join("test.md.j2");
        fs::write(&template_file, content).unwrap();
        template_file
    }

    #[test]
    fn test_template_rendering() {
        let temp_dir = TempDir::new().unwrap();
        let template_content = "# {{ name }}\n\nDesc: {{ desc }}\nUser: {{ user }}";
        create_test_template(temp_dir.path(), "default", template_content);

        let mut renderer = TemplateRenderer::new(temp_dir.path().to_str().unwrap()).unwrap();

        let context = TemplateContext {
            name: "test-branch".to_string(),
            desc: "Test description".to_string(),
            template: "default".to_string(),
            base_dir: "/test/base".to_string(),
            pr_dir_abs: "/test/base/test-branch".to_string(),
            pr_dir_rel: "test-branch".to_string(),
            git_info: None,
        };

        let config = Config::default();
        let result = renderer
            .render_template("default", "test.md.j2", &context, &config)
            .unwrap();

        assert!(result.contains("# test-branch"));
        assert!(result.contains("Desc: Test description"));
    }

    #[test]
    fn test_template_with_filters() {
        let temp_dir = TempDir::new().unwrap();
        let template_content = "{{ desc | slugify }}";
        create_test_template(temp_dir.path(), "default", template_content);

        let mut renderer = TemplateRenderer::new(temp_dir.path().to_str().unwrap()).unwrap();

        let context = TemplateContext {
            name: "test".to_string(),
            desc: "Hello World Test".to_string(),
            template: "default".to_string(),
            base_dir: "/test".to_string(),
            pr_dir_abs: "/test/test".to_string(),
            pr_dir_rel: "test".to_string(),
            git_info: None,
        };

        let config = Config::default();
        let result = renderer
            .render_template("default", "test.md.j2", &context, &config)
            .unwrap();

        assert_eq!(result.trim(), "hello-world-test");
    }
}
