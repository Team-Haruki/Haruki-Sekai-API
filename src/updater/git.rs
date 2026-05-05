use std::fmt::Write as _;
use std::path::Path;
use std::process::{Command, Output};

use tracing::info;

use crate::config::{GitConfig, GitSigningFormat};
use crate::error::AppError;

pub struct GitHelper {
    pub username: String,
    pub email: String,
    pub password: String,
    pub sign_commits: bool,
    pub signing_format: GitSigningFormat,
    pub signing_key: String,
    pub signing_program: String,
    pub proxy: Option<String>,
}

impl GitHelper {
    pub fn new(config: &GitConfig, proxy: Option<String>) -> Self {
        Self {
            username: config.username.clone(),
            email: config.email.clone(),
            password: config.password.clone(),
            sign_commits: config.sign_commits,
            signing_format: config.signing_format,
            signing_key: config.signing_key.clone(),
            signing_program: config.signing_program.clone(),
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

        if !self.has_changes(repo_path)? {
            if !self.has_unpushed_commits(repo_path)? {
                info!("No changes to commit or push");
                return Ok(false);
            }
            info!("Found unpushed commits");
        } else {
            self.stage_all(repo_path)?;
            let commit_msg = format!("Sekai master data version {}", data_version);
            self.commit(
                repo_path,
                &commit_msg,
                "Haruki Sekai Master Update Bot",
                "no-reply@mail.seiunx.com",
            )?;
            info!("Committed changes: {}", commit_msg);
        }

        self.push(repo_path)?;
        info!("Pushed changes successfully");
        Ok(true)
    }

    fn git(&self, repo_path: &str) -> Command {
        let mut command = Command::new("git");
        command.arg("-C").arg(repo_path);
        command
    }

    fn run(&self, mut command: Command, action: &str) -> Result<Output, AppError> {
        let output = command
            .output()
            .map_err(|e| AppError::NetworkError(format!("Failed to run git {}: {}", action, e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let detail = stderr.trim();
            let detail = if detail.is_empty() {
                output.status.to_string()
            } else {
                detail.to_string()
            };
            return Err(AppError::NetworkError(format!(
                "git {} failed: {}",
                action, detail
            )));
        }

        Ok(output)
    }

    fn has_changes(&self, repo_path: &str) -> Result<bool, AppError> {
        let mut cmd = self.git(repo_path);
        cmd.args(["status", "--porcelain"]);
        let output = self.run(cmd, "status")?;
        Ok(!output.stdout.is_empty())
    }

    fn has_unpushed_commits(&self, repo_path: &str) -> Result<bool, AppError> {
        // Errors when there's no upstream or HEAD is unborn — both mean "nothing to push".
        let output = self
            .git(repo_path)
            .args(["rev-list", "--count", "@{u}..HEAD"])
            .output()
            .map_err(|e| AppError::NetworkError(format!("Failed to run git rev-list: {}", e)))?;
        if !output.status.success() {
            return Ok(false);
        }
        let count: u32 = String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse()
            .unwrap_or(0);
        Ok(count > 0)
    }

    fn stage_all(&self, repo_path: &str) -> Result<(), AppError> {
        let mut cmd = self.git(repo_path);
        cmd.args(["add", "-A"]);
        self.run(cmd, "add")?;
        Ok(())
    }

    fn commit(
        &self,
        repo_path: &str,
        message: &str,
        author_name: &str,
        author_email: &str,
    ) -> Result<(), AppError> {
        let mut cmd = self.git(repo_path);

        if self.sign_commits {
            let (format, program_key) = match self.signing_format {
                GitSigningFormat::Gpg => ("openpgp", "gpg.program"),
                GitSigningFormat::Ssh => ("ssh", "gpg.ssh.program"),
            };
            cmd.arg("-c").arg(format!("gpg.format={}", format));
            if !self.signing_program.is_empty() {
                cmd.arg("-c")
                    .arg(format!("{}={}", program_key, self.signing_program));
            }
            if !self.signing_key.is_empty() {
                cmd.arg("-c")
                    .arg(format!("user.signingkey={}", self.signing_key));
            }
            cmd.arg("commit").arg("-S");
        } else {
            cmd.arg("commit");
        }

        cmd.arg("-m")
            .arg(message)
            .env("GIT_AUTHOR_NAME", author_name)
            .env("GIT_AUTHOR_EMAIL", author_email)
            .env("GIT_COMMITTER_NAME", &self.username)
            .env("GIT_COMMITTER_EMAIL", &self.email);

        self.run(cmd, "commit")?;
        Ok(())
    }

    fn push(&self, repo_path: &str) -> Result<(), AppError> {
        let head = self.run(
            {
                let mut c = self.git(repo_path);
                c.args(["symbolic-ref", "--short", "HEAD"]);
                c
            },
            "symbolic-ref",
        )?;
        let branch = String::from_utf8_lossy(&head.stdout).trim().to_string();

        let push_target = if self.password.is_empty() {
            "origin".to_string()
        } else {
            let remote = self.run(
                {
                    let mut c = self.git(repo_path);
                    c.args(["remote", "get-url", "origin"]);
                    c
                },
                "remote get-url",
            )?;
            let url = String::from_utf8_lossy(&remote.stdout).trim().to_string();
            inject_credentials(&url, &self.username, &self.password)?
        };

        let refspec = format!("refs/heads/{branch}:refs/heads/{branch}");

        let mut cmd = self.git(repo_path);
        if let Some(proxy) = self.proxy.as_deref() {
            if !proxy.is_empty() {
                cmd.arg("-c").arg(format!("http.proxy={}", proxy));
            }
        }
        cmd.args(["push", &push_target, &refspec])
            .env("GIT_TERMINAL_PROMPT", "0");

        self.run(cmd, "push")?;
        Ok(())
    }
}

fn inject_credentials(url: &str, username: &str, password: &str) -> Result<String, AppError> {
    let scheme_end = if let Some(rest) = url.strip_prefix("https://") {
        ("https://", rest)
    } else if let Some(rest) = url.strip_prefix("http://") {
        ("http://", rest)
    } else {
        return Err(AppError::NetworkError(format!(
            "Cannot inject credentials into non-HTTP(S) remote URL: {}",
            url
        )));
    };
    let (scheme, rest) = scheme_end;
    let rest = match rest.find('@') {
        Some(at) if !rest[..at].contains('/') => &rest[at + 1..],
        _ => rest,
    };
    Ok(format!(
        "{}{}:{}@{}",
        scheme,
        pct_encode(username),
        pct_encode(password),
        rest
    ))
}

fn pct_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if matches!(b, b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            let _ = write!(out, "%{:02X}", b);
        }
    }
    out
}
