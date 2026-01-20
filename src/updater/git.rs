use std::path::Path;
use std::process::Command;

use tracing::info;

use crate::config::GitConfig;
use crate::error::AppError;

pub struct GitHelper {
    pub username: String,
    pub email: String,
    pub password: String,
    pub proxy: Option<String>,
}

impl GitHelper {
    pub fn new(config: &GitConfig, proxy: Option<String>) -> Self {
        Self {
            username: config.username.clone(),
            email: config.email.clone(),
            password: config.password.clone(),
            proxy,
        }
    }

    pub fn push_changes(&self, repo_path: &str, data_version: &str) -> Result<bool, AppError> {
        let path = Path::new(repo_path);
        if !path.exists() {
            return Err(AppError::ParseError(format!(
                "Repository path does not exist: {}",
                repo_path
            )));
        }
        let status = self.run_git(repo_path, &["status", "--porcelain"])?;
        if status.trim().is_empty() {
            let unpushed = self.run_git(repo_path, &["log", "@{u}..", "--oneline"]);
            if unpushed.is_err()
                || unpushed
                    .as_ref()
                    .map(|s| s.trim().is_empty())
                    .unwrap_or(true)
            {
                info!("No changes to commit or push");
                return Ok(false);
            }
            info!("Found unpushed commits");
        } else {
            self.run_git(repo_path, &["add", "-A"])?;
            let commit_msg = format!("Sekai master data version {}", data_version);
            let author = "Haruki Sekai Master Update Bot <no-reply@mail.seiunx.com>";
            self.run_git(
                repo_path,
                &[
                    "-c",
                    &format!("user.name={}", self.username),
                    "-c",
                    &format!("user.email={}", self.email),
                    "commit",
                    "--author",
                    author,
                    "-m",
                    &commit_msg,
                ],
            )?;
            info!("Committed changes: {}", commit_msg);
        }
        self.push_to_remote(repo_path)?;
        info!("Pushed changes successfully");
        Ok(true)
    }

    fn push_to_remote(&self, repo_path: &str) -> Result<(), AppError> {
        let branch = self.run_git(repo_path, &["rev-parse", "--abbrev-ref", "HEAD"])?;
        let branch = branch.trim();
        if !self.password.is_empty() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let askpass_script =
                    format!("#!/bin/sh\necho '{}'", self.password.replace("'", "'\\''"));
                let askpass_path = "/tmp/git-askpass.sh";
                std::fs::write(askpass_path, &askpass_script)
                    .map_err(|e| AppError::ParseError(format!("Failed to write askpass: {}", e)))?;
                std::fs::set_permissions(askpass_path, std::fs::Permissions::from_mode(0o700))
                    .map_err(|e| {
                        AppError::ParseError(format!("Failed to set permissions: {}", e))
                    })?;
                let mut cmd = Command::new("git");
                cmd.current_dir(repo_path)
                    .args(["push", "origin", branch])
                    .env("GIT_ASKPASS", askpass_path);
                if let Some(ref proxy) = self.proxy {
                    if !proxy.is_empty() {
                        cmd.env("HTTP_PROXY", proxy).env("HTTPS_PROXY", proxy);
                    }
                }
                let output = cmd.output().map_err(|e| {
                    AppError::NetworkError(format!("Failed to run git push: {}", e))
                })?;
                let _ = std::fs::remove_file(askpass_path);
                if !output.status.success() {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    if !stderr.contains("already up-to-date")
                        && !stderr.contains("Everything up-to-date")
                    {
                        return Err(AppError::NetworkError(format!(
                            "Git push failed: {}",
                            stderr
                        )));
                    }
                }
            }
            #[cfg(windows)]
            {
                let askpass_script = format!("@echo off\necho {}", self.password);
                let askpass_path = std::env::temp_dir().join("git-askpass.bat");
                std::fs::write(&askpass_path, &askpass_script)
                    .map_err(|e| AppError::ParseError(format!("Failed to write askpass: {}", e)))?;
                let mut cmd = Command::new("git");
                cmd.current_dir(repo_path)
                    .args(["push", "origin", branch])
                    .env("GIT_ASKPASS", &askpass_path);
                if let Some(ref proxy) = self.proxy {
                    if !proxy.is_empty() {
                        cmd.env("HTTP_PROXY", proxy).env("HTTPS_PROXY", proxy);
                    }
                }
                let output = cmd.output().map_err(|e| {
                    AppError::NetworkError(format!("Failed to run git push: {}", e))
                })?;
                let _ = std::fs::remove_file(&askpass_path);
                if !output.status.success() {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    if !stderr.contains("already up-to-date")
                        && !stderr.contains("Everything up-to-date")
                    {
                        return Err(AppError::NetworkError(format!(
                            "Git push failed: {}",
                            stderr
                        )));
                    }
                }
            }
        } else {
            self.run_git(repo_path, &["push", "origin", branch])?;
        }
        Ok(())
    }

    fn run_git(&self, repo_path: &str, args: &[&str]) -> Result<String, AppError> {
        let mut cmd = Command::new("git");
        cmd.current_dir(repo_path).args(args);
        if let Some(ref proxy) = self.proxy {
            if !proxy.is_empty() {
                cmd.env("HTTP_PROXY", proxy).env("HTTPS_PROXY", proxy);
            }
        }
        let output = cmd
            .output()
            .map_err(|e| AppError::NetworkError(format!("Failed to run git: {}", e)))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if !stderr.contains("nothing to commit")
                && !stderr.contains("already up-to-date")
                && !stderr.contains("Everything up-to-date")
            {
                return Err(AppError::NetworkError(format!(
                    "Git command failed: {}",
                    stderr
                )));
            }
        }
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }
}
