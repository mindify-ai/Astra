//! Git operations via libgit2: worktrees per mission agent, commits, diffs and merges.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use astra_protocol::{DiffFile, DiffLine, DiffLineKind, DiffStats};
use git2::{
    BranchType, DiffFormat, DiffOptions, IndexAddOption, Oid, Repository, Signature,
    build::CheckoutBuilder,
};

/// Git errors.
#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error("git: {0}")]
    Git(#[from] git2::Error),
    #[error("merge conflict merging {branch} into {into}")]
    Conflict { branch: String, into: String },
    #[error("{0}")]
    Other(String),
}

type Result<T> = std::result::Result<T, GitError>;

fn signature() -> Result<Signature<'static>> {
    Ok(Signature::now("Astra", "astra@localhost")?)
}

/// Expand a leading `~/`.
pub fn expand_home(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/")
        && let Some(home) = std::env::var_os("HOME")
    {
        return PathBuf::from(home).join(rest);
    }
    PathBuf::from(path)
}

fn worktree_name(branch: &str) -> String {
    let mut s: String = branch
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    s.truncate(80);
    s
}

/// Create (or reuse) a worktree at `dir` on branch `branch`, creating the branch from `base` if needed.
pub fn create_worktree(repo_path: &Path, base: &str, branch: &str, dir: &Path) -> Result<PathBuf> {
    let repo = Repository::open(repo_path)?;
    if dir.join(".git").exists() {
        let wt = Repository::open(dir)?;
        if wt
            .head()
            .ok()
            .and_then(|h| h.shorthand().ok().map(str::to_string))
            .as_deref()
            == Some(branch)
        {
            return Ok(dir.to_path_buf());
        }
        return Err(GitError::Other(format!(
            "{} exists and is not on {branch}",
            dir.display()
        )));
    }
    forget_lost_worktrees(&repo, branch, dir)?;
    // An empty directory left behind is not a worktree, and git won't create one over it.
    if dir.is_dir() && std::fs::read_dir(dir).is_ok_and(|mut d| d.next().is_none()) {
        std::fs::remove_dir(dir).map_err(|e| GitError::Other(e.to_string()))?;
    }
    let br = match repo.find_branch(branch, BranchType::Local) {
        Ok(b) => b,
        Err(_) => repo.branch(
            branch,
            &repo.revparse_single(base)?.peel_to_commit()?,
            false,
        )?,
    };
    if let Some(parent) = dir.parent() {
        std::fs::create_dir_all(parent).map_err(|e| GitError::Other(e.to_string()))?;
    }
    let mut name = worktree_name(branch);
    let existing: Vec<String> = repo
        .worktrees()?
        .iter()
        .flatten()
        .flatten()
        .map(str::to_string)
        .collect();
    let mut n = 1;
    while existing.contains(&name) {
        n += 1;
        name = format!("{}-{n}", worktree_name(branch));
    }
    let reference = br.into_reference();
    let mut opts = git2::WorktreeAddOptions::new();
    opts.reference(Some(&reference));
    repo.worktree(&name, dir, Some(&opts))?;
    Ok(dir.to_path_buf())
}

/// `path` with symlinks resolved as far as it exists (a lost worktree's directory doesn't).
fn resolved(path: &Path) -> PathBuf {
    if let Ok(p) = path.canonicalize() {
        return p;
    }
    match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) => resolved(parent).join(name),
        _ => path.to_path_buf(),
    }
}

/// Forgets worktrees whose directory is gone from disk (deleted by hand or by a cleanup) and that were at
/// `dir` or on `branch`. The repository still lists them, and a listed worktree keeps its branch checked out,
/// so the branch could not get a new one. What `git worktree prune` does, for the ones in the way.
fn forget_lost_worktrees(repo: &Repository, branch: &str, dir: &Path) -> Result<()> {
    let target = resolved(dir);
    let head = format!("ref: refs/heads/{branch}");
    for name in repo.worktrees()?.iter().flatten().flatten() {
        let Ok(wt) = repo.find_worktree(name) else {
            continue;
        };
        if wt.validate().is_ok() {
            continue;
        }
        let on_branch =
            std::fs::read_to_string(repo.commondir().join("worktrees").join(name).join("HEAD"))
                .is_ok_and(|h| h.trim() == head);
        if on_branch || resolved(wt.path()) == target {
            // The default options only prune a worktree that is not valid and not locked.
            wt.prune(None)?;
        }
    }
    Ok(())
}

/// Remove the worktree checked out at `dir` (its branch is kept).
pub fn remove_worktree(repo_path: &Path, dir: &Path) -> Result<()> {
    let repo = Repository::open(repo_path)?;
    let target = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    for name in repo.worktrees()?.iter().flatten().flatten() {
        let wt = repo.find_worktree(name)?;
        let p = wt
            .path()
            .canonicalize()
            .unwrap_or_else(|_| wt.path().to_path_buf());
        if p == target {
            let _ = std::fs::remove_dir_all(&p);
            let mut opts = git2::WorktreePruneOptions::new();
            opts.valid(true).working_tree(true);
            wt.prune(Some(&mut opts))?;
        }
    }
    Ok(())
}

/// Stage everything in a worktree and commit it. Returns `None` when there was nothing to commit.
pub fn commit_all(worktree: &Path, message: &str) -> Result<Option<Oid>> {
    let repo = Repository::open(worktree)?;
    let mut index = repo.index()?;
    index.add_all(["*"].iter(), IndexAddOption::DEFAULT, None)?;
    index.update_all(["*"].iter(), None)?;
    index.write()?;
    let tree_id = index.write_tree()?;
    let head = repo.head()?.peel_to_commit()?;
    if head.tree_id() == tree_id {
        return Ok(None);
    }
    let tree = repo.find_tree(tree_id)?;
    let sig = signature()?;
    Ok(Some(repo.commit(
        Some("HEAD"),
        &sig,
        &sig,
        message,
        &tree,
        &[&head],
    )?))
}

fn branch_commit<'r>(repo: &'r Repository, name: &str) -> Result<git2::Commit<'r>> {
    Ok(repo
        .find_branch(name, BranchType::Local)?
        .get()
        .peel_to_commit()?)
}

/// Diff of `branch` against its merge base with `base`: stats and unified diff lines.
pub fn diff_branch(
    repo_path: &Path,
    base: &str,
    branch: &str,
) -> Result<(DiffStats, Vec<DiffFile>)> {
    let repo = Repository::open(repo_path)?;
    let base_c = branch_commit(&repo, base)?;
    let head_c = branch_commit(&repo, branch)?;
    let mb = repo.merge_base(base_c.id(), head_c.id())?;
    let old_tree = repo.find_commit(mb)?.tree()?;
    let new_tree = head_c.tree()?;
    let mut opts = DiffOptions::new();
    opts.context_lines(3);
    let diff = repo.diff_tree_to_tree(Some(&old_tree), Some(&new_tree), Some(&mut opts))?;
    let st = diff.stats()?;
    let to_u32 = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
    let stats = DiffStats {
        files: to_u32(st.files_changed()),
        additions: to_u32(st.insertions()),
        deletions: to_u32(st.deletions()),
    };
    let files: RefCell<Vec<DiffFile>> = RefCell::new(Vec::new());
    diff.print(DiffFormat::Patch, |delta, _hunk, line| {
        let path = delta
            .new_file()
            .path()
            .or_else(|| delta.old_file().path())
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let kind = match line.origin() {
            '+' => DiffLineKind::Add,
            '-' => DiffLineKind::Del,
            ' ' => DiffLineKind::Ctx,
            _ => return true,
        };
        let text = String::from_utf8_lossy(line.content())
            .trim_end_matches(['\n', '\r'])
            .to_string();
        let mut files = files.borrow_mut();
        if files.last().map(|f| f.path != path).unwrap_or(true) {
            files.push(DiffFile {
                path,
                lines: vec![],
            });
        }
        if let Some(f) = files.last_mut() {
            f.lines.push(DiffLine { kind, text });
        }
        true
    })?;
    Ok((stats, files.into_inner()))
}

/// How a merge went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeOutcome {
    UpToDate,
    FastForward(Oid),
    MergeCommit(Oid),
}

/// Merge `branch` into `into` in the main repository (fast-forward when possible, else a merge commit).
/// If `into` is checked out, the working tree is updated safely (fails rather than clobbering local edits).
pub fn merge_branch(
    repo_path: &Path,
    branch: &str,
    into: &str,
    message: &str,
) -> Result<MergeOutcome> {
    let repo = Repository::open(repo_path)?;
    let ours = branch_commit(&repo, into)?;
    let theirs = branch_commit(&repo, branch)?;
    if ours.id() == theirs.id() || repo.graph_descendant_of(ours.id(), theirs.id())? {
        return Ok(MergeOutcome::UpToDate);
    }
    let head_is_into = !repo.is_bare()
        && repo
            .head()
            .ok()
            .and_then(|h| h.shorthand().ok().map(str::to_string))
            .as_deref()
            == Some(into);
    let mut into_ref = repo.find_branch(into, BranchType::Local)?.into_reference();
    if repo.graph_descendant_of(theirs.id(), ours.id())? {
        if head_is_into {
            let mut co = CheckoutBuilder::new();
            co.safe();
            repo.checkout_tree(theirs.as_object(), Some(&mut co))?;
        }
        into_ref.set_target(theirs.id(), message)?;
        return Ok(MergeOutcome::FastForward(theirs.id()));
    }
    let mut idx = repo.merge_commits(&ours, &theirs, None)?;
    if idx.has_conflicts() {
        return Err(GitError::Conflict {
            branch: branch.into(),
            into: into.into(),
        });
    }
    let tree_id = idx.write_tree_to(&repo)?;
    let tree = repo.find_tree(tree_id)?;
    if head_is_into {
        let mut co = CheckoutBuilder::new();
        co.safe();
        repo.checkout_tree(tree.as_object(), Some(&mut co))?;
    }
    let sig = signature()?;
    let refname = into_ref.name()?.to_string();
    let oid = repo.commit(
        Some(&refname),
        &sig,
        &sig,
        message,
        &tree,
        &[&ours, &theirs],
    )?;
    Ok(MergeOutcome::MergeCommit(oid))
}

/// Whether a local branch exists.
pub fn branch_exists(repo_path: &Path, branch: &str) -> bool {
    Repository::open(repo_path)
        .and_then(|r| r.find_branch(branch, BranchType::Local).map(|_| ()))
        .is_ok()
}

/// Read a file at the tip of a branch.
pub fn read_file_at(repo_path: &Path, branch: &str, path: &str) -> Result<Option<String>> {
    let repo = Repository::open(repo_path)?;
    let tree = branch_commit(&repo, branch)?.tree()?;
    match tree.get_path(Path::new(path)) {
        Ok(entry) => {
            let blob = repo.find_blob(entry.id())?;
            Ok(Some(String::from_utf8_lossy(blob.content()).into_owned()))
        }
        Err(_) => Ok(None),
    }
}

/// Initialize a repository with one commit on `branch` (tests, demos).
pub fn init_repo(path: &Path, branch: &str, files: &[(&str, &str)]) -> Result<()> {
    let repo = Repository::init(path)?;
    for (p, content) in files {
        let full = path.join(p);
        if let Some(dir) = full.parent() {
            std::fs::create_dir_all(dir).map_err(|e| GitError::Other(e.to_string()))?;
        }
        std::fs::write(full, content).map_err(|e| GitError::Other(e.to_string()))?;
    }
    let mut index = repo.index()?;
    index.add_all(["*"].iter(), IndexAddOption::DEFAULT, None)?;
    index.write()?;
    let tree = repo.find_tree(index.write_tree()?)?;
    let sig = signature()?;
    let refname = format!("refs/heads/{branch}");
    repo.commit(Some(&refname), &sig, &sig, "initial", &tree, &[])?;
    repo.set_head(&refname)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worktree_commit_diff_merge() {
        let tmp = tempfile::tempdir().expect("tmp");
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("mkdir");
        init_repo(&repo, "main", &[("README.md", "hello\nworld\n")]).expect("init");

        let a =
            create_worktree(&repo, "main", "astra/x-a", &tmp.path().join("wt/a")).expect("wt a");
        let b =
            create_worktree(&repo, "main", "astra/x-b", &tmp.path().join("wt/b")).expect("wt b");
        // Idempotent.
        create_worktree(&repo, "main", "astra/x-a", &tmp.path().join("wt/a")).expect("wt a again");
        std::fs::write(a.join("a.txt"), "A\n").expect("write");
        std::fs::write(a.join("README.md"), "hello\nastra\n").expect("write");
        std::fs::write(b.join("b.txt"), "B\n").expect("write");
        assert!(commit_all(&a, "a").expect("commit").is_some());
        assert!(commit_all(&a, "noop").expect("commit").is_none());
        commit_all(&b, "b").expect("commit");

        let (stats, files) = diff_branch(&repo, "main", "astra/x-a").expect("diff");
        assert_eq!(
            stats,
            DiffStats {
                files: 2,
                additions: 2,
                deletions: 1
            }
        );
        let readme = files
            .iter()
            .find(|f| f.path == "README.md")
            .expect("readme");
        assert!(readme.lines.contains(&DiffLine {
            kind: DiffLineKind::Del,
            text: "world".into()
        }));
        assert!(readme.lines.contains(&DiffLine {
            kind: DiffLineKind::Add,
            text: "astra".into()
        }));
        assert!(readme.lines.contains(&DiffLine {
            kind: DiffLineKind::Ctx,
            text: "hello".into()
        }));

        assert!(matches!(
            merge_branch(&repo, "astra/x-a", "main", "merge a").expect("ff"),
            MergeOutcome::FastForward(_)
        ));
        assert_eq!(
            std::fs::read_to_string(repo.join("a.txt")).expect("checked out"),
            "A\n"
        );
        assert!(matches!(
            merge_branch(&repo, "astra/x-b", "main", "merge b").expect("merge"),
            MergeOutcome::MergeCommit(_)
        ));
        assert_eq!(
            read_file_at(&repo, "main", "b.txt")
                .expect("read")
                .as_deref(),
            Some("B\n")
        );
        assert_eq!(
            merge_branch(&repo, "astra/x-b", "main", "again").expect("noop"),
            MergeOutcome::UpToDate
        );

        remove_worktree(&repo, &a).expect("remove");
        assert!(!a.exists());
        assert!(branch_exists(&repo, "astra/x-a"));
    }

    #[test]
    fn a_lost_worktree_is_created_again_on_its_branch() {
        let tmp = tempfile::tempdir().expect("tmp");
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("mkdir");
        init_repo(&repo, "main", &[("README.md", "hello\n")]).expect("init");
        let dir = tmp.path().join("wt/a");
        let a = create_worktree(&repo, "main", "astra/x", &dir).expect("wt");
        std::fs::write(a.join("a.txt"), "A\n").expect("write");
        commit_all(&a, "a").expect("commit");

        // Deleted behind git's back: the repository still lists it, holding the branch.
        std::fs::remove_dir_all(&a).expect("delete");
        let again = create_worktree(&repo, "main", "astra/x", &dir).expect("wt again");
        assert_eq!(
            std::fs::read_to_string(again.join("a.txt")).expect("the branch's work"),
            "A\n"
        );

        // Lost again, with an empty directory left where it was, and wanted somewhere else this time.
        std::fs::remove_dir_all(&again).expect("delete");
        std::fs::create_dir_all(&again).expect("empty dir");
        let moved = tmp.path().join("elsewhere/a");
        create_worktree(&repo, "main", "astra/x", &moved).expect("wt elsewhere");
        assert!(moved.join("a.txt").exists());
        // A worktree that is still there keeps its branch.
        assert!(create_worktree(&repo, "main", "astra/x", &dir).is_err());
    }
}
