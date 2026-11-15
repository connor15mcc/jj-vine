use std::path::PathBuf;

use dialoguer::Input;
use owo_colors::OwoColorize as _;

use crate::{
    commands::init::{
        Remotes,
        apply_token_source,
        get_config,
        parse_forge_url,
        prompt_token,
        set_config,
    },
    error::Result,
};

/// Initialize GitHub-specific configuration.
#[expect(clippy::single_call_fn, reason = "important")]
#[expect(
    clippy::too_many_lines,
    reason = "meh, only +9. hey, if you're revisiting this function, consider fixing this? :)"
)]
pub fn init(repo_path: impl Into<PathBuf>, remotes: Option<&Remotes>) -> Result<()> {
    let repo_path = repo_path.into();
    let existing_host = get_config(&repo_path, "jj-vine.github.host");
    let existing_project = get_config(&repo_path, "jj-vine.github.project");
    let existing_target_project = get_config(&repo_path, "jj-vine.github.targetProject");

    let remotes = remotes.as_ref();
    let forge = match remotes {
        Some(Remotes {
            target_forge: Some(forge),
            ..
        }) => Some(forge),
        _ => None,
    };

    let default_host = existing_host
        .or(forge.map(|f| f.host.clone()))
        .unwrap_or_else(|| "https://api.github.com".to_owned());
    let default_project = existing_project.or(forge.map(|f| f.project.clone()));

    let github_host = Input::<String>::new()
        .with_prompt(format!(
            "{} {}",
            "GitHub API URL (e.g. https://api.github.com)".bold(),
            "jj-vine.github.host".dimmed()
        ))
        .default(default_host)
        .interact_text()?;

    let github_project = if let Some(project) = default_project {
        Input::<String>::new()
            .with_prompt(format!(
                "{} {}",
                "GitHub repository (owner/repo)".bold(),
                "jj-vine.github.project".dimmed()
            ))
            .default(project)
            .interact_text()?
    } else {
        Input::<String>::new()
            .with_prompt(format!(
                "{} {}",
                "GitHub repository (owner/repo)".bold(),
                "jj-vine.github.project".dimmed()
            ))
            .interact_text()?
    };

    let github_target_project = Input::<String>::new()
        .with_prompt(format!(
            "{} {}",
            "Target repository for PRs (upstream, leave blank for same as source repository)"
                .bold(),
            "jj-vine.github.targetProject".dimmed()
        ))
        .with_initial_text(
            existing_target_project
                .or(remotes.and_then(|f| {
                    f.upstream
                        .as_ref()
                        .and_then(|u| parse_forge_url(u))
                        .map(|f| f.project)
                }))
                .or(remotes.and_then(|f| parse_forge_url(&f.origin).map(|f| f.project)))
                .unwrap_or(github_project.clone()),
        )
        .allow_empty(true)
        .interact_text()?;

    let default_token_command = which::which("gh").is_ok().then_some("gh auth token");
    let github_token = prompt_token(
        &repo_path,
        "jj-vine.github",
        "GitHub",
        default_token_command,
        || {
            println!();
            println!("{}", "Personal Access Token required scopes:".yellow());
            println!(
                "  {} {}",
                "•".yellow(),
                "repo (for creating/updating pull requests)".dimmed()
            );
            println!();
            println!(
                "  {}",
                format!(
                    "Create token at: {}/settings/tokens/new",
                    if github_host == "https://api.github.com" {
                        "https://github.com"
                    } else {
                        github_host.strip_suffix("/api/v3").unwrap_or(&github_host)
                    }
                )
                .dimmed()
            );
            println!();
        },
    )?;

    set_config(&repo_path, "jj-vine.github.host", &github_host)?;
    set_config(&repo_path, "jj-vine.github.project", &github_project)?;
    if !github_target_project.is_empty() {
        set_config(
            &repo_path,
            "jj-vine.github.targetProject",
            &github_target_project,
        )?;
    }
    apply_token_source(&repo_path, "jj-vine.github", github_token)?;

    Ok(())
}
