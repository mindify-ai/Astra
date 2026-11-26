//! Local git operations. Merge previews use an in-memory index; network access is HTTPS to GitHub only.
use git2::{BranchType, Cred, ErrorCode, IndexAddOption, Repository, RepositoryOpenFlags};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Mutex;

// Serialize shell git operations, including reads against commits/checkouts in flight.
static ACCESS: Mutex<()> = Mutex::new(());
fn error(e: git2::Error) -> String {
    if e.code() == ErrorCode::Auth {
        "signedOut".into()
    } else if matches!(
        e.class(),
        git2::ErrorClass::Net | git2::ErrorClass::Http | git2::ErrorClass::Ssl
    ) {
        "network".into()
    } else {
        e.message().to_string()
    }
}
fn open(root: &str) -> Result<Repository, String> {
    let path = Path::new(root);
    if !path.is_absolute() {
        return Err("The repository folder must be an absolute path.".into());
    }
    let canonical = path.canonicalize().map_err(|e| e.to_string())?;
    if !canonical.is_dir() {
        return Err("The repository root must be a folder.".into());
    }
    // Never discover a parent repository when the selected folder isn't its root.
    let repo = Repository::open_ext(&canonical, RepositoryOpenFlags::NO_SEARCH, &[] as &[&Path])
        .map_err(error)?;
    let workdir = repo
        .workdir()
        .ok_or("Bare repositories are not supported.")?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if canonical != workdir {
        return Err("Choose the repository's root folder.".into());
    }
    Ok(repo)
}
fn branch_ref(branch: &str) -> Result<String, String> {
    let name = format!("refs/heads/{branch}");
    if !git2::Reference::is_valid_name(&name) {
        return Err("Invalid branch name.".into());
    }
    Ok(name)
}
fn branch_commit<'a>(repo: &'a Repository, branch: &str) -> Result<git2::Commit<'a>, String> {
    repo.find_reference(&branch_ref(branch)?)
        .and_then(|r| r.peel_to_commit())
        .map_err(error)
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    branch: String,
    upstream: Option<String>,
    ahead: usize,
    behind: usize,
    changed: Vec<Change>,
}
#[derive(Debug, Serialize)]
pub struct Change {
    path: String,
    state: String,
}
#[derive(Debug, Serialize)]
pub struct Commit {
    id: String,
    message: String,
    author: String,
    at: String,
}
#[derive(Debug, Serialize)]
pub struct MergeCheck {
    clean: bool,
    conflicts: Vec<String>,
}
fn preview(repo: &Repository, branch: &str, into: &str) -> Result<MergeCheck, String> {
    let ours = branch_commit(repo, branch)?;
    let theirs = branch_commit(repo, into)?;
    preview_commits(repo, &ours, &theirs)
}
fn preview_commits(
    repo: &Repository,
    ours: &git2::Commit<'_>,
    theirs: &git2::Commit<'_>,
) -> Result<MergeCheck, String> {
    let index = repo.merge_commits(ours, theirs, None).map_err(error)?;
    let mut conflicts = Vec::new();
    if index.has_conflicts() {
        for conflict in index.conflicts().map_err(error)? {
            let conflict = conflict.map_err(error)?;
            for entry in [conflict.ancestor, conflict.our, conflict.their]
                .into_iter()
                .flatten()
            {
                conflicts.push(String::from_utf8_lossy(&entry.path).into_owned());
            }
        }
    }
    conflicts.sort();
    conflicts.dedup();
    Ok(MergeCheck {
        clean: conflicts.is_empty(),
        conflicts,
    })
}
fn status(repo: &Repository) -> Result<Status, String> {
    let branch = repo
        .head()
        .ok()
        .and_then(|h| h.shorthand().ok().map(str::to_owned))
        .unwrap_or_else(|| "Unborn branch".into());
    let upstream = repo
        .find_branch(&branch, BranchType::Local)
        .ok()
        .and_then(|b| b.upstream().ok());
    let upstream_name = upstream
        .as_ref()
        .and_then(|b| b.name().ok().flatten().map(str::to_owned));
    let (ahead, behind) = repo
        .head()
        .ok()
        .and_then(|h| h.target())
        .zip(upstream.as_ref().and_then(|b| b.get().target()))
        .and_then(|(head, up)| repo.graph_ahead_behind(head, up).ok())
        .unwrap_or((0, 0));
    let mut opts = git2::StatusOptions::new();
    opts.include_untracked(true).recurse_untracked_dirs(true);
    let changed = repo
        .statuses(Some(&mut opts))
        .map_err(error)?
        .iter()
        .map(|entry| {
            let flags = entry.status();
            let state = if flags.is_conflicted() {
                "conflict"
            } else if flags.is_wt_deleted() || flags.is_index_deleted() {
                "deleted"
            } else if flags.is_wt_new() || flags.is_index_new() {
                "new"
            } else if flags.is_wt_renamed() || flags.is_index_renamed() {
                "renamed"
            } else {
                "modified"
            };
            Change {
                path: String::from_utf8_lossy(entry.path_bytes()).into_owned(),
                state: state.into(),
            }
        })
        .collect();
    Ok(Status {
        branch,
        upstream: upstream_name,
        ahead,
        behind,
        changed,
    })
}
fn clean(repo: &Repository) -> Result<(), String> {
    if repo.state() != git2::RepositoryState::Clean || !status(repo)?.changed.is_empty() {
        return Err("dirty".into());
    }
    Ok(())
}
fn commit(repo: &Repository, message: &str) -> Result<(), String> {
    if message.trim().is_empty() {
        return Err("Enter a commit message.".into());
    }
    if repo.state() != git2::RepositoryState::Clean {
        return Err("Finish the existing merge in Git first.".into());
    }
    if status(repo)?.changed.is_empty() {
        return Err("No changes to commit.".into());
    }
    let mut index = repo.index().map_err(error)?;
    if index.has_conflicts() {
        return Err("Resolve conflicts before committing.".into());
    }
    index
        .add_all(["*"], IndexAddOption::DEFAULT, None)
        .map_err(error)?;
    index.update_all(["*"], None).map_err(error)?;
    index.write().map_err(error)?;
    let tree = repo
        .find_tree(index.write_tree().map_err(error)?)
        .map_err(error)?;
    let sig = repo.signature().map_err(error)?;
    let parent = match repo.head() {
        Ok(head) => Some(head.peel_to_commit().map_err(error)?),
        Err(e) if e.code() == ErrorCode::UnbornBranch || e.code() == ErrorCode::NotFound => None,
        Err(e) => return Err(error(e)),
    };
    repo.commit(
        Some("HEAD"),
        &sig,
        &sig,
        message.trim(),
        &tree,
        &parent.iter().collect::<Vec<_>>(),
    )
    .map_err(error)?;
    Ok(())
}
/// A token may only reach a GitHub HTTPS origin, never an arbitrary configured remote.
fn github_repo(url: &str) -> Result<String, String> {
    let parsed = reqwest::Url::parse(url).map_err(|_| "Use an HTTPS GitHub remote.")?;
    if parsed.scheme() != "https"
        || parsed.host_str() != Some("github.com")
        || parsed.port().is_some()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err("Use an HTTPS GitHub remote.".into());
    }
    let full = parsed
        .path()
        .trim_start_matches('/')
        .trim_end_matches(".git");
    if !crate::github::valid_full_name(full) {
        return Err("Invalid GitHub remote.".into());
    }
    Ok(full.into())
}
fn callbacks(token: String) -> git2::RemoteCallbacks<'static> {
    let mut callbacks = git2::RemoteCallbacks::new();
    callbacks.credentials(move |url, _, _| {
        github_repo(url)
            .map_err(|_| git2::Error::from_str("Refusing credentials for this remote."))?;
        Cred::userpass_plaintext("x-access-token", &token)
    });
    callbacks
}
fn upstream(repo: &Repository) -> Result<(String, String, String), String> {
    let head = repo.head().map_err(error)?;
    if !head.is_branch() {
        return Err("Check out a branch before pulling or pushing.".into());
    }
    let name = head.name().map_err(error)?.to_owned();
    let remote = repo
        .branch_upstream_remote(&name)
        .map_err(|_| "noUpstream")?;
    let target = repo
        .branch_upstream_merge(&name)
        .map_err(|_| "noUpstream")?;
    Ok((
        name,
        remote.as_str().map_err(error)?.into(),
        target.as_str().map_err(error)?.into(),
    ))
}
fn push_refspec(source: &str, target: &str) -> Result<String, String> {
    if !source.starts_with("refs/heads/")
        || !target.starts_with("refs/heads/")
        || !git2::Reference::is_valid_name(source)
        || !git2::Reference::is_valid_name(target)
    {
        return Err("Invalid push branch.".into());
    }
    Ok(format!("{source}:{target}"))
}
fn push(repo: &Repository) -> Result<(), String> {
    let (source, remote_name, target) = upstream(repo)?;
    let mut remote = repo.find_remote(&remote_name).map_err(error)?;
    let url = remote
        .pushurl()
        .map_err(error)?
        .unwrap_or(remote.url().map_err(error)?);
    github_repo(url)?;
    let token = crate::github_token::get().ok_or("signedOut")?;
    let mut cb = callbacks(token);
    cb.push_update_reference(|_, status| match status {
        Some(message) => Err(git2::Error::from_str(message)),
        None => Ok(()),
    });
    let mut options = git2::PushOptions::new();
    options
        .remote_callbacks(cb)
        .follow_redirects(git2::RemoteRedirect::None);
    // No '+' refspec and no force option; the server rejects non-fast-forward updates.
    remote
        .push(&[push_refspec(&source, &target)?], Some(&mut options))
        .map_err(error)
}
fn pull(repo: &Repository) -> Result<MergeCheck, String> {
    clean(repo)?;
    let (source, remote_name, target) = upstream(repo)?;
    let mut remote = repo.find_remote(&remote_name).map_err(error)?;
    github_repo(remote.url().map_err(error)?)?;
    let token = crate::github_token::get().ok_or("signedOut")?;
    let mut opts = git2::FetchOptions::new();
    opts.remote_callbacks(callbacks(token))
        .follow_redirects(git2::RemoteRedirect::None);
    remote
        .fetch(&[&target], Some(&mut opts), None)
        .map_err(error)?;
    let fetched = repo
        .find_reference("FETCH_HEAD")
        .and_then(|r| r.peel_to_commit())
        .map_err(error)?;
    let head = repo
        .find_reference(&source)
        .and_then(|r| r.peel_to_commit())
        .map_err(error)?;
    if fetched.id() == head.id()
        || repo
            .graph_descendant_of(head.id(), fetched.id())
            .map_err(error)?
    {
        return Ok(MergeCheck {
            clean: true,
            conflicts: vec![],
        });
    }
    if !repo
        .graph_descendant_of(fetched.id(), head.id())
        .map_err(error)?
    {
        let preview = preview_commits(repo, &head, &fetched)?;
        if !preview.clean {
            return Ok(preview);
        }
        return Err(
            "Branches have diverged. Merge the upstream branch in Git before pulling.".into(),
        );
    }
    clean(repo)?;
    // Safe checkout refuses to overwrite local edits; move the ref only after checkout succeeds.
    repo.checkout_tree(
        fetched.as_object(),
        Some(git2::build::CheckoutBuilder::new().safe()),
    )
    .map_err(error)?;
    repo.find_reference(&source)
        .and_then(|mut r| r.set_target(fetched.id(), "Astra fast-forward pull"))
        .map_err(error)?;
    Ok(MergeCheck {
        clean: true,
        conflicts: vec![],
    })
}
async fn blocking<T: Send + 'static>(
    root: String,
    f: impl FnOnce(Repository) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let _lock = ACCESS.lock().map_err(|e| e.to_string())?;
        f(open(&root)?)
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn git_status(root: String) -> Result<Status, String> {
    blocking(root, |r| status(&r)).await
}
#[tauri::command]
pub async fn git_branches(root: String) -> Result<Vec<String>, String> {
    blocking(root, |r| {
        r.branches(Some(BranchType::Local))
            .map_err(error)?
            .map(|b| {
                b.map_err(error).and_then(|(b, _)| {
                    b.name()
                        .map_err(error)?
                        .map(str::to_owned)
                        .ok_or("Invalid branch.".into())
                })
            })
            .collect()
    })
    .await
}
#[tauri::command]
pub async fn git_tags(root: String) -> Result<Vec<String>, String> {
    blocking(root, |r| {
        let mut tags = Vec::new();
        for name in r.tag_names(None).map_err(error)?.iter() {
            let Some(name) = name.map_err(error)? else {
                continue;
            };
            let reference = r
                .find_reference(&format!("refs/tags/{name}"))
                .map_err(error)?;
            let time = reference
                .peel_to_tag()
                .map(|t| t.tagger().map(|s| s.when().seconds()).unwrap_or(0))
                .or_else(|_| reference.peel_to_commit().map(|c| c.time().seconds()))
                .unwrap_or(0);
            tags.push((time, name.to_string()));
        }
        tags.sort_by(|a, b| b.cmp(a));
        Ok(tags.into_iter().map(|(_, name)| name).collect())
    })
    .await
}
#[tauri::command]
pub async fn git_log(root: String, limit: usize) -> Result<Vec<Commit>, String> {
    blocking(root, move |r| {
        let mut walk = r.revwalk().map_err(error)?;
        match walk.push_head() {
            Ok(()) => {}
            Err(e) if e.code() == ErrorCode::UnbornBranch || e.code() == ErrorCode::NotFound => {
                return Ok(vec![]);
            }
            Err(e) => return Err(error(e)),
        }
        walk.set_sorting(git2::Sort::TIME).map_err(error)?;
        walk.take(limit.min(100))
            .map(|id| {
                let c = r.find_commit(id.map_err(error)?).map_err(error)?;
                let date = time::OffsetDateTime::from_unix_timestamp(c.time().seconds())
                    .map_err(|e| e.to_string())?
                    .format(&time::format_description::well_known::Rfc3339)
                    .map_err(|e| e.to_string())?;
                Ok(Commit {
                    id: c.id().to_string(),
                    message: c.summary().map_err(error)?.unwrap_or("").into(),
                    author: c.author().name().unwrap_or("").into(),
                    at: date,
                })
            })
            .collect()
    })
    .await
}
#[tauri::command]
pub async fn git_check_merge(
    root: String,
    branch: String,
    into: String,
) -> Result<MergeCheck, String> {
    blocking(root, move |r| preview(&r, &branch, &into)).await
}
#[tauri::command]
pub async fn git_commit(root: String, message: String) -> Result<(), String> {
    blocking(root, move |r| commit(&r, &message)).await
}
#[tauri::command]
pub async fn git_pull(root: String) -> Result<MergeCheck, String> {
    blocking(root, |r| pull(&r)).await
}
#[tauri::command]
pub async fn git_push(root: String) -> Result<(), String> {
    blocking(root, |r| push(&r)).await
}
#[tauri::command]
pub async fn git_create_tag(root: String, name: String, message: String) -> Result<(), String> {
    blocking(root, move |r| {
        if !git2::Reference::is_valid_name(&format!("refs/tags/{name}")) {
            return Err("Invalid version tag.".into());
        }
        r.tag(
            &name,
            &r.head()
                .and_then(|h| h.peel(git2::ObjectType::Commit))
                .map_err(error)?,
            &r.signature().map_err(error)?,
            &message,
            false,
        )
        .map_err(error)?;
        Ok(())
    })
    .await
}
#[derive(Deserialize)]
pub struct PullRequestDraft {
    head: String,
    base: String,
    title: String,
    body: String,
}
async fn full_name(root: String) -> Result<String, String> {
    blocking(root, |r| {
        github_repo(
            r.find_remote("origin")
                .map_err(error)?
                .url()
                .map_err(error)?,
        )
    })
    .await
}
#[tauri::command]
pub async fn git_pull_requests(root: String) -> Result<Vec<crate::github::PullRequest>, String> {
    let full = full_name(root).await?;
    let token = crate::github_token::get().ok_or("signedOut")?;
    crate::github::pull_requests(&token, &full)
        .await
        .map_err(str::to_string)
}
#[tauri::command]
pub async fn git_create_pull_request(
    root: String,
    draft: PullRequestDraft,
) -> Result<crate::github::PullRequest, String> {
    branch_ref(&draft.head)?;
    branch_ref(&draft.base)?;
    let full = full_name(root).await?;
    let token = crate::github_token::get().ok_or("signedOut")?;
    crate::github::create_pull_request(
        &token,
        &full,
        &draft.head,
        &draft.base,
        &draft.title,
        &draft.body,
    )
    .await
    .map_err(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Tmp(std::path::PathBuf);
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn setup() -> (Tmp, Repository) {
        let dir = std::env::temp_dir().join(format!(
            "astra-git-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let repo = Repository::init(&dir).unwrap();
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "Test").unwrap();
        config.set_str("user.email", "test@example.com").unwrap();
        std::fs::write(dir.join("file.txt"), "base\n").unwrap();
        commit(&repo, "Initial").unwrap();
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("main", &head, true).unwrap();
        repo.set_head("refs/heads/main").unwrap();
        drop(head);
        (Tmp(dir), repo)
    }
    #[test]
    fn confines_root_without_discovering_parent() {
        let (tmp, _) = setup();
        assert!(open("relative").is_err());
        assert!(open(tmp.0.join("file.txt").to_str().unwrap()).is_err());
        std::fs::create_dir(tmp.0.join("sub")).unwrap();
        assert!(open(tmp.0.join("sub").to_str().unwrap()).is_err());
        assert!(open(tmp.0.to_str().unwrap()).is_ok());
    }
    #[test]
    fn merge_checks_are_in_memory() {
        let (tmp, repo) = setup();
        repo.branch(
            "feature",
            &repo.head().unwrap().peel_to_commit().unwrap(),
            false,
        )
        .unwrap();
        repo.set_head("refs/heads/feature").unwrap();
        std::fs::write(tmp.0.join("file.txt"), "feature\n").unwrap();
        commit(&repo, "Feature").unwrap();
        repo.set_head("refs/heads/main").unwrap();
        repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
            .unwrap();
        std::fs::write(tmp.0.join("file.txt"), "main\n").unwrap();
        commit(&repo, "Main").unwrap();
        let before = std::fs::read(tmp.0.join(".git/index")).unwrap();
        let head = repo.head().unwrap().target();
        let check = preview(&repo, "feature", "main").unwrap();
        assert!(!check.clean);
        assert_eq!(check.conflicts, ["file.txt"]);
        assert!(preview(&repo, "main", "main").unwrap().clean);
        assert_eq!(std::fs::read(tmp.0.join("file.txt")).unwrap(), b"main\n");
        assert_eq!(std::fs::read(tmp.0.join(".git/index")).unwrap(), before);
        assert_eq!(repo.head().unwrap().target(), head);
        assert_eq!(repo.state(), git2::RepositoryState::Clean);
    }
    #[test]
    fn pushes_never_force_and_credentials_stay_on_github() {
        assert_eq!(
            push_refspec("refs/heads/main", "refs/heads/trunk").unwrap(),
            "refs/heads/main:refs/heads/trunk"
        );
        assert!(push_refspec("+refs/heads/main", "refs/heads/main").is_err());
        assert_eq!(
            github_repo("https://github.com/org/repo.git").unwrap(),
            "org/repo"
        );
        for bad in [
            "http://github.com/org/repo",
            "https://github.com.evil.test/org/repo",
            "https://evil.test/org/repo",
            "https://token@github.com/org/repo",
            "../repo",
        ] {
            assert!(github_repo(bad).is_err());
        }
    }
}
