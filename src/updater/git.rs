use std::path::Path;

use git2::{
    Cred, DiffOptions, ErrorCode, IndexAddOption, ProxyOptions, PushOptions, RemoteCallbacks,
    Repository, Signature, StatusOptions,
};
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

        let repo = Repository::open(path)
            .map_err(|e| AppError::NetworkError(format!("Failed to open git repository: {}", e)))?;

        if !self.has_changes(&repo)? {
            if !self.has_unpushed_commits(&repo)? {
                info!("No changes to commit or push");
                return Ok(false);
            }
            info!("Found unpushed commits");
        } else {
            self.stage_all(&repo)?;

            let commit_msg = format!("Sekai master data version {}", data_version);
            let author = "Haruki Sekai Master Update Bot";
            let author_email = "no-reply@mail.seiunx.com";
            self.commit(&repo, &commit_msg, author, author_email)?;
            info!("Committed changes: {}", commit_msg);
        }

        self.push(&repo)?;
        info!("Pushed changes successfully");
        Ok(true)
    }

    fn has_changes(&self, repo: &Repository) -> Result<bool, AppError> {
        let mut opts = StatusOptions::new();
        opts.include_untracked(true)
            .recurse_untracked_dirs(true)
            .include_ignored(false);

        let statuses = repo
            .statuses(Some(&mut opts))
            .map_err(|e| AppError::NetworkError(format!("Failed to get git status: {}", e)))?;

        Ok(!statuses.is_empty())
    }

    fn has_unpushed_commits(&self, repo: &Repository) -> Result<bool, AppError> {
        let head = match repo.head() {
            Ok(h) => h,
            Err(e) if e.code() == ErrorCode::UnbornBranch => return Ok(false),
            Err(e) => return Err(AppError::NetworkError(format!("Failed to get HEAD: {}", e))),
        };

        let local_oid = head.target().ok_or_else(|| {
            AppError::NetworkError("HEAD does not point to a valid commit".to_string())
        })?;

        let branch = repo
            .find_branch(head.shorthand().unwrap_or("main"), git2::BranchType::Local)
            .map_err(|e| AppError::NetworkError(format!("Failed to find local branch: {}", e)))?;

        let upstream = match branch.upstream() {
            Ok(u) => u,
            Err(_) => return Ok(false),
        };

        let upstream_oid = upstream.get().target().ok_or_else(|| {
            AppError::NetworkError("Upstream does not point to a valid commit".to_string())
        })?;

        let (ahead, _behind) = repo
            .graph_ahead_behind(local_oid, upstream_oid)
            .map_err(|e| AppError::NetworkError(format!("Failed to compare commits: {}", e)))?;

        Ok(ahead > 0)
    }

    fn stage_all(&self, repo: &Repository) -> Result<(), AppError> {
        let mut index = repo
            .index()
            .map_err(|e| AppError::NetworkError(format!("Failed to get index: {}", e)))?;

        index
            .add_all(["*"].iter(), IndexAddOption::DEFAULT, None)
            .map_err(|e| AppError::NetworkError(format!("Failed to stage files: {}", e)))?;

        // Also handle deletions: update index to remove files that no longer exist on disk
        let mut diff_opts = DiffOptions::new();
        let diff = repo
            .diff_index_to_workdir(Some(&index), Some(&mut diff_opts))
            .map_err(|e| AppError::NetworkError(format!("Failed to diff index: {}", e)))?;

        let deleted_paths: Vec<String> = diff
            .deltas()
            .filter(|d| d.status() == git2::Delta::Deleted)
            .filter_map(|d| d.old_file().path())
            .map(|p| p.to_string_lossy().to_string())
            .collect();

        for path in &deleted_paths {
            let _ = index.remove_path(Path::new(path));
        }

        index
            .write()
            .map_err(|e| AppError::NetworkError(format!("Failed to write index: {}", e)))?;

        Ok(())
    }

    fn commit(
        &self,
        repo: &Repository,
        message: &str,
        author_name: &str,
        author_email: &str,
    ) -> Result<(), AppError> {
        let sig_author = Signature::now(author_name, author_email).map_err(|e| {
            AppError::NetworkError(format!("Failed to create author signature: {}", e))
        })?;

        let sig_committer = Signature::now(&self.username, &self.email).map_err(|e| {
            AppError::NetworkError(format!("Failed to create committer signature: {}", e))
        })?;

        let mut index = repo
            .index()
            .map_err(|e| AppError::NetworkError(format!("Failed to get index: {}", e)))?;

        let tree_oid = index
            .write_tree()
            .map_err(|e| AppError::NetworkError(format!("Failed to write tree: {}", e)))?;

        let tree = repo
            .find_tree(tree_oid)
            .map_err(|e| AppError::NetworkError(format!("Failed to find tree: {}", e)))?;

        let parent_commit = match repo.head() {
            Ok(head) => {
                let oid = head
                    .target()
                    .ok_or_else(|| AppError::NetworkError("HEAD has no target".to_string()))?;
                Some(repo.find_commit(oid).map_err(|e| {
                    AppError::NetworkError(format!("Failed to find parent commit: {}", e))
                })?)
            }
            Err(e) if e.code() == ErrorCode::UnbornBranch => None,
            Err(e) => return Err(AppError::NetworkError(format!("Failed to get HEAD: {}", e))),
        };

        let parents: Vec<&git2::Commit> = parent_commit.iter().collect();

        repo.commit(
            Some("HEAD"),
            &sig_author,
            &sig_committer,
            message,
            &tree,
            &parents,
        )
        .map_err(|e| AppError::NetworkError(format!("Failed to commit: {}", e)))?;

        Ok(())
    }

    fn push(&self, repo: &Repository) -> Result<(), AppError> {
        let head = repo
            .head()
            .map_err(|e| AppError::NetworkError(format!("Failed to get HEAD: {}", e)))?;

        let branch_name = head.shorthand().unwrap_or("main");
        let refspec = format!("refs/heads/{}:refs/heads/{}", branch_name, branch_name);

        let mut remote = repo.find_remote("origin").map_err(|e| {
            AppError::NetworkError(format!("Failed to find remote 'origin': {}", e))
        })?;

        let mut callbacks = RemoteCallbacks::new();

        if !self.password.is_empty() {
            let username = self.username.clone();
            let password = self.password.clone();
            callbacks.credentials(move |_url, _username_from_url, _allowed_types| {
                Cred::userpass_plaintext(&username, &password)
            });
        }

        // Report push errors via the callback
        callbacks.push_update_reference(|refname, status| {
            if let Some(msg) = status {
                Err(git2::Error::from_str(&format!(
                    "Failed to push ref {}: {}",
                    refname, msg
                )))
            } else {
                Ok(())
            }
        });

        let mut push_opts = PushOptions::new();
        push_opts.remote_callbacks(callbacks);

        if let Some(ref proxy_url) = self.proxy {
            if !proxy_url.is_empty() {
                let mut proxy_opts = ProxyOptions::new();
                proxy_opts.url(proxy_url);
                push_opts.proxy_options(proxy_opts);
            }
        }

        remote
            .push(&[&refspec], Some(&mut push_opts))
            .map_err(|e| {
                let msg = e.message().to_string();
                if msg.contains("up-to-date") {
                    return AppError::NetworkError("Already up-to-date".to_string());
                }
                AppError::NetworkError(format!("Git push failed: {}", msg))
            })?;

        Ok(())
    }
}
