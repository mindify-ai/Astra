//! File access for the in-app code editor: list a folder, read a text file, write a text file. Paths
//! arrive relative to a repository folder (`root`) and are confined to it: `..`, absolute paths and
//! symlinks that resolve outside the canonical root are refused, and `.git` is off limits (writing there
//! could plant hooks). Listings skip `.git`, `node_modules` and `target`. Reads stop at 2 MB and refuse
//! anything that isn't UTF-8 text.

use std::ffi::OsStr;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use serde::Serialize;

/// The largest file the editor opens or saves.
const MAX_BYTES: u64 = 2 * 1024 * 1024;
/// A listing stops here; a folder this large isn't browsed by hand.
const MAX_ENTRIES: usize = 5_000;
/// Never listed: version control internals and build output.
const HIDDEN: &[&str] = &[".git", "node_modules", "target"];

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    File,
    Dir,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct Entry {
    pub name: String,
    /// Relative to the root, `/`-separated.
    pub path: String,
    pub kind: Kind,
}

fn is_git(name: &OsStr) -> bool {
    name.to_str()
        .is_some_and(|n| n.eq_ignore_ascii_case(".git"))
}

/// The canonical repository folder. It must be an absolute path to an existing folder.
fn root_dir(root: &str) -> Result<PathBuf, String> {
    let r = Path::new(root);
    if !r.is_absolute() {
        return Err("The repository folder must be an absolute path.".into());
    }
    let c = r
        .canonicalize()
        .map_err(|e| format!("Cannot open {root}: {e}"))?;
    if !c.is_dir() {
        return Err(format!("{root} is not a folder."));
    }
    Ok(c)
}

/// Checks a relative path lexically: plain names only (no `..`, root or drive prefix), never `.git`.
fn relative(rel: &str) -> Result<PathBuf, String> {
    let mut out = PathBuf::new();
    for c in Path::new(rel).components() {
        match c {
            Component::Normal(n) if is_git(n) => {
                return Err("Files inside .git are off limits.".into());
            }
            Component::Normal(n) => out.push(n),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(format!("{rel} leaves the repository."));
            }
        }
    }
    Ok(out)
}

/// Refuses a resolved (canonical) path outside `root`, or inside its `.git`.
fn ensure_inside(root: &Path, resolved: &Path) -> Result<(), String> {
    let rest = resolved
        .strip_prefix(root)
        .map_err(|_| "That path leaves the repository.".to_string())?;
    if rest.components().any(|c| is_git(c.as_os_str())) {
        return Err("Files inside .git are off limits.".into());
    }
    Ok(())
}

/// Resolves an existing `rel` inside `root`, following symlinks, and refuses anything that lands outside.
fn confine(root: &Path, rel: &str) -> Result<PathBuf, String> {
    let resolved = root
        .join(relative(rel)?)
        .canonicalize()
        .map_err(|e| format!("Cannot open {rel}: {e}"))?;
    ensure_inside(root, &resolved)?;
    Ok(resolved)
}

/// Where to write `rel`: its folder must already exist inside `root`, and an existing file (or a symlink to
/// one) must resolve inside `root` too.
fn confine_for_write(root: &Path, rel: &str) -> Result<PathBuf, String> {
    let lexical = relative(rel)?;
    let name = lexical
        .file_name()
        .ok_or_else(|| "A file path is required.".to_string())?
        .to_owned();
    let parent_rel = lexical.parent().unwrap_or(Path::new(""));
    let parent = root
        .join(parent_rel)
        .canonicalize()
        .map_err(|e| format!("Cannot open the folder of {rel}: {e}"))?;
    ensure_inside(root, &parent)?;
    if !parent.is_dir() {
        return Err(format!("The folder of {rel} doesn't exist."));
    }
    let target = parent.join(name);
    if std::fs::symlink_metadata(&target).is_ok() {
        let resolved = target
            .canonicalize()
            .map_err(|e| format!("Cannot open {rel}: {e}"))?;
        ensure_inside(root, &resolved)?;
        if !resolved.is_file() {
            return Err(format!("{rel} is not a file."));
        }
        return Ok(resolved);
    }
    Ok(target)
}

pub fn list(root: &str, dir: &str) -> Result<Vec<Entry>, String> {
    let root = root_dir(root)?;
    let folder = confine(&root, dir)?;
    if !folder.is_dir() {
        return Err(format!("{dir} is not a folder."));
    }
    // Entries keep the path as asked for (through any in-repo symlink), so a later call resolves it again.
    let base = relative(dir)?
        .components()
        .filter_map(|c| c.as_os_str().to_str().map(str::to_string))
        .collect::<Vec<_>>()
        .join("/");
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&folder).map_err(|e| format!("Cannot list {dir}: {e}"))? {
        let Ok(entry) = entry else { continue };
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        if HIDDEN.iter().any(|h| name.eq_ignore_ascii_case(h)) {
            continue;
        }
        // Broken links and links that point out of the repository are left out.
        let Ok(real) = entry.path().canonicalize() else {
            continue;
        };
        if ensure_inside(&root, &real).is_err() {
            continue;
        }
        let kind = if real.is_dir() {
            Kind::Dir
        } else if real.is_file() {
            Kind::File
        } else {
            continue;
        };
        let path = if base.is_empty() {
            name.clone()
        } else {
            format!("{base}/{name}")
        };
        out.push(Entry { name, path, kind });
        if out.len() >= MAX_ENTRIES {
            break;
        }
    }
    out.sort_by(|a, b| {
        (a.kind != Kind::Dir)
            .cmp(&(b.kind != Kind::Dir))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(out)
}

pub fn read(root: &str, path: &str) -> Result<String, String> {
    let root = root_dir(root)?;
    let file = confine(&root, path)?;
    let meta = std::fs::metadata(&file).map_err(|e| format!("Cannot open {path}: {e}"))?;
    if !meta.is_file() {
        return Err(format!("{path} is not a file."));
    }
    if meta.len() > MAX_BYTES {
        return Err(format!("{path} is larger than 2 MB."));
    }
    let mut bytes = Vec::with_capacity(meta.len() as usize);
    std::fs::File::open(&file)
        .and_then(|f| f.take(MAX_BYTES + 1).read_to_end(&mut bytes))
        .map_err(|e| format!("Cannot read {path}: {e}"))?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(format!("{path} is larger than 2 MB."));
    }
    String::from_utf8(bytes).map_err(|_| format!("{path} is not UTF-8 text."))
}

pub fn write(root: &str, path: &str, text: &str) -> Result<(), String> {
    if text.len() as u64 > MAX_BYTES {
        return Err(format!("{path} would be larger than 2 MB."));
    }
    let root = root_dir(root)?;
    let target = confine_for_write(&root, path)?;
    std::fs::write(&target, text).map_err(|e| format!("Cannot save {path}: {e}"))
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn files_list(root: String, dir: String) -> Result<Vec<Entry>, String> {
    blocking(move || list(&root, &dir)).await
}

#[tauri::command]
pub async fn files_read(root: String, path: String) -> Result<String, String> {
    blocking(move || read(&root, &path)).await
}

#[tauri::command]
pub async fn files_write(root: String, path: String, text: String) -> Result<(), String> {
    blocking(move || write(&root, &path, &text)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// A scratch folder holding `repo/` and an `outside/` sibling; removed on drop.
    struct Tmp(PathBuf);

    impl Tmp {
        fn new(tag: &str) -> Self {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let dir = std::env::temp_dir()
                .join(format!("astra-files-{tag}-{}-{nanos}", std::process::id()));
            fs::create_dir_all(dir.join("repo/src")).unwrap();
            fs::create_dir_all(dir.join("outside")).unwrap();
            fs::write(dir.join("repo/src/main.ts"), "export {}\n").unwrap();
            fs::write(dir.join("outside/secret.txt"), "secret").unwrap();
            Tmp(dir.canonicalize().unwrap())
        }
        fn root(&self) -> String {
            self.0.join("repo").to_string_lossy().into_owned()
        }
    }

    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn relative_paths_stay_lexically_inside() {
        assert_eq!(
            relative("src/./main.ts").unwrap(),
            PathBuf::from("src/main.ts")
        );
        assert_eq!(relative("").unwrap(), PathBuf::new());
        assert!(relative("../x").is_err());
        assert!(relative("src/../../x").is_err());
        assert!(relative("src/..").is_err());
        assert!(relative("/etc/passwd").is_err());
        assert!(relative(".git/config").is_err());
        assert!(relative("sub/.GIT/hooks/pre-commit").is_err());
    }

    #[test]
    fn root_must_be_an_absolute_folder() {
        let t = Tmp::new("root");
        assert!(list("relative/path", "").is_err());
        let file = t.0.join("repo/src/main.ts");
        assert!(list(&file.to_string_lossy(), "").is_err());
        assert!(list(&t.root(), "").is_ok());
    }

    #[test]
    fn refuses_parent_escapes() {
        let t = Tmp::new("parent");
        let root = t.root();
        assert!(read(&root, "../outside/secret.txt").is_err());
        assert!(read(&root, "src/../../outside/secret.txt").is_err());
        let abs = t.0.join("outside/secret.txt");
        assert!(read(&root, &abs.to_string_lossy()).is_err());
        assert!(list(&root, "..").is_err());
        assert!(write(&root, "../outside/secret.txt", "x").is_err());
        assert!(write(&root, "../planted.txt", "x").is_err());
        assert_eq!(fs::read_to_string(&abs).unwrap(), "secret");
        assert!(!t.0.join("planted.txt").exists());
    }

    #[test]
    fn git_is_off_limits() {
        let t = Tmp::new("git");
        let root = t.root();
        fs::create_dir_all(t.0.join("repo/.git/hooks")).unwrap();
        fs::write(t.0.join("repo/.git/config"), "[core]").unwrap();
        assert!(read(&root, ".git/config").is_err());
        assert!(write(&root, ".git/hooks/pre-commit", "#!/bin/sh").is_err());
        assert!(list(&root, ".git").is_err());
        assert!(!t.0.join("repo/.git/hooks/pre-commit").exists());
    }

    #[cfg(unix)]
    #[test]
    fn refuses_symlinks_that_leave_the_root() {
        use std::os::unix::fs::symlink;
        let t = Tmp::new("symlink");
        let root = t.root();
        symlink(t.0.join("outside"), t.0.join("repo/out-dir")).unwrap();
        symlink(t.0.join("outside/secret.txt"), t.0.join("repo/out-file")).unwrap();
        symlink(t.0.join("repo/src"), t.0.join("repo/in-dir")).unwrap();
        fs::create_dir_all(t.0.join("repo/.git")).unwrap();
        symlink(t.0.join("repo/.git"), t.0.join("repo/git-link")).unwrap();

        assert!(read(&root, "out-file").is_err());
        assert!(read(&root, "out-dir/secret.txt").is_err());
        assert!(list(&root, "out-dir").is_err());
        assert!(write(&root, "out-file", "x").is_err());
        assert!(write(&root, "out-dir/new.txt", "x").is_err());
        assert!(list(&root, "git-link").is_err());
        assert!(write(&root, "git-link/config", "x").is_err());
        assert_eq!(
            fs::read_to_string(t.0.join("outside/secret.txt")).unwrap(),
            "secret"
        );
        assert!(!t.0.join("outside/new.txt").exists());

        // Links that stay inside work; links out (and into .git) are not listed.
        assert_eq!(read(&root, "in-dir/main.ts").unwrap(), "export {}\n");
        let names: Vec<String> = list(&root, "")
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert_eq!(names, vec!["in-dir", "src"]);
        assert_eq!(
            list(&root, "in-dir").unwrap(),
            vec![Entry {
                name: "main.ts".into(),
                path: "in-dir/main.ts".into(),
                kind: Kind::File
            }]
        );
    }

    #[test]
    fn listing_hides_build_and_vcs_folders() {
        let t = Tmp::new("list");
        let root = t.root();
        for d in [".git", "node_modules/x", "target/debug", "docs"] {
            fs::create_dir_all(t.0.join("repo").join(d)).unwrap();
        }
        fs::write(t.0.join("repo/README.md"), "# Hi").unwrap();
        fs::write(t.0.join("repo/b.txt"), "").unwrap();
        let got = list(&root, "").unwrap();
        let rows: Vec<(&str, &str, &Kind)> = got
            .iter()
            .map(|e| (e.name.as_str(), e.path.as_str(), &e.kind))
            .collect();
        assert_eq!(
            rows,
            vec![
                ("docs", "docs", &Kind::Dir),
                ("src", "src", &Kind::Dir),
                ("b.txt", "b.txt", &Kind::File),
                ("README.md", "README.md", &Kind::File),
            ]
        );
        assert_eq!(list(&root, "./src").unwrap()[0].path, "src/main.ts");
    }

    #[test]
    fn reads_only_small_utf8_files() {
        let t = Tmp::new("read");
        let root = t.root();
        assert_eq!(read(&root, "src/main.ts").unwrap(), "export {}\n");
        fs::write(t.0.join("repo/big.txt"), vec![b'a'; MAX_BYTES as usize + 1]).unwrap();
        assert!(read(&root, "big.txt").unwrap_err().contains("2 MB"));
        fs::write(t.0.join("repo/bin.dat"), [0xff, 0xfe, 0x00, 0x80]).unwrap();
        assert!(read(&root, "bin.dat").unwrap_err().contains("UTF-8"));
        assert!(read(&root, "src").is_err());
        assert!(read(&root, "missing.ts").is_err());
    }

    #[test]
    fn writes_inside_existing_folders() {
        let t = Tmp::new("write");
        let root = t.root();
        write(&root, "src/main.ts", "export const a = 1\n").unwrap();
        assert_eq!(read(&root, "src/main.ts").unwrap(), "export const a = 1\n");
        write(&root, "src/new.ts", "new").unwrap();
        assert_eq!(read(&root, "src/new.ts").unwrap(), "new");
        assert!(write(&root, "nope/new.ts", "x").is_err());
        assert!(write(&root, "src", "x").is_err());
        assert!(write(&root, "", "x").is_err());
        let big = "a".repeat(MAX_BYTES as usize + 1);
        assert!(write(&root, "src/big.ts", &big).is_err());
    }
}
