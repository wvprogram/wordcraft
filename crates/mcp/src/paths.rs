//! Which files an agent may read and write through the MCP server.
//!
//! Text the agent reads can come from the documents it opens, so a hostile document can try to
//! steer it ("open ~/notes/passwords.txt", "export over ~/.bashrc"). Every path an agent passes to
//! a command (any `"path"` string in its parameters, at any depth) must resolve inside one folder:
//! `wordcraft-cli mcp --root DIR`, the current folder by default. Paths are resolved (`..` and
//! symbolic links) before the check and handed to the backend as absolute paths.

use std::path::{Component, Path, PathBuf};

use serde_json::Value;

/// The folder an agent's paths must stay inside.
#[derive(Clone, Debug)]
pub struct PathPolicy {
    /// Canonical root, or why there is none (every path is then refused).
    root: Result<PathBuf, String>,
}

impl PathPolicy {
    /// Confine paths to `dir` (resolved now; it must exist).
    pub fn within(dir: &Path) -> Result<Self, String> {
        let root = dir.canonicalize().map_err(|e| format!("--root {}: {e}", dir.display()))?;
        if !root.is_dir() {
            return Err(format!("--root {}: not a folder", dir.display()));
        }
        Ok(Self { root: Ok(root) })
    }

    /// Confine paths to the current folder. If it can't be resolved, every path is refused.
    pub fn current_dir() -> Self {
        let root = std::env::current_dir().map_err(|e| e.to_string()).and_then(|d| d.canonicalize().map_err(|e| e.to_string()));
        Self { root: root.map_err(|e| format!("no folder for files ({e}); start the server with --root DIR")) }
    }

    /// The folder, when there is one.
    pub fn root(&self) -> Option<&Path> {
        self.root.as_deref().ok()
    }

    /// `path` resolved against the root, as an absolute path inside it; an error otherwise. A
    /// file that doesn't exist yet (a save) is allowed when its folder is inside the root.
    pub fn check(&self, path: &str) -> Result<PathBuf, String> {
        let root = self.root.as_ref().map_err(Clone::clone)?;
        if path.trim().is_empty() {
            return Err("empty path".into());
        }
        let p = Path::new(path);
        let joined = if p.is_absolute() { p.to_path_buf() } else { root.join(p) };
        let resolved = match joined.canonicalize() {
            Ok(r) => r,
            Err(_) => {
                // A new file: its folder must exist and resolve inside the root, and the name
                // must be a plain file name (not `..`).
                let name = match joined.components().next_back() {
                    Some(Component::Normal(n)) => n.to_os_string(),
                    _ => return Err(format!("{path}: not a file path")),
                };
                let parent = joined.parent().ok_or_else(|| format!("{path}: not a file path"))?;
                parent.canonicalize().map_err(|e| format!("{path}: {e}"))?.join(name)
            }
        };
        if resolved.starts_with(root) {
            Ok(resolved)
        } else {
            Err(format!("{path}: outside the folder this MCP server may use ({}); start it with --root to choose another", root.display()))
        }
    }

    /// Check every `"path"` string in `params` (at any depth) and replace it with its resolved
    /// absolute form. Fails on the first path outside the root.
    pub fn confine(&self, mut params: Value) -> Result<Value, String> {
        self.walk(&mut params, 0)?;
        Ok(params)
    }

    fn walk(&self, v: &mut Value, depth: usize) -> Result<(), String> {
        if depth > 64 {
            return Err("parameters nested too deeply".into());
        }
        match v {
            Value::Object(m) => {
                for (k, x) in m.iter_mut() {
                    if k == "path"
                        && let Value::String(s) = x
                    {
                        *s = self.check(s)?.to_string_lossy().into_owned();
                    } else {
                        self.walk(x, depth + 1)?;
                    }
                }
            }
            Value::Array(a) => {
                for x in a {
                    self.walk(x, depth + 1)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn temp_tree() -> (PathBuf, PathBuf) {
        let base = std::env::temp_dir().join(format!("wordcraft-mcp-paths-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let root = base.join("work");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::create_dir_all(base.join("private")).unwrap();
        std::fs::write(root.join("in.txt"), "ok").unwrap();
        std::fs::write(base.join("private/passwords.txt"), "secret").unwrap();
        (base, root)
    }

    #[test]
    fn paths_must_stay_inside_the_root() {
        let (base, root) = temp_tree();
        let p = PathPolicy::within(&root).unwrap();
        let root_c = root.canonicalize().unwrap();
        assert_eq!(p.check("in.txt").unwrap(), root_c.join("in.txt"), "relative to the root");
        assert_eq!(p.check(&root.join("in.txt").to_string_lossy()).unwrap(), root_c.join("in.txt"));
        assert_eq!(p.check("sub/new.docx").unwrap(), root_c.join("sub/new.docx"), "a new file in a folder inside");
        for bad in [
            base.join("private/passwords.txt").to_string_lossy().into_owned(),
            "../private/passwords.txt".to_string(),
            "sub/../../private/passwords.txt".to_string(),
            "../private/new.docx".to_string(),
            "no-such-folder/x.docx".to_string(),
            "/".to_string(),
            "".to_string(),
            "..".to_string(),
        ] {
            assert!(p.check(&bad).is_err(), "{bad:?}");
        }
        #[cfg(unix)]
        {
            // A symbolic link inside the root that points out of it is outside.
            std::os::unix::fs::symlink(base.join("private"), root.join("link")).unwrap();
            assert!(p.check("link/passwords.txt").is_err());
            assert!(p.check("link/new.docx").is_err());
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn every_path_parameter_is_checked_and_made_absolute() {
        let (base, root) = temp_tree();
        let p = PathPolicy::within(&root).unwrap();
        let out = p.confine(json!({"path": "in.txt", "other": "../x", "nested": [{"path": "sub/a.png"}]})).unwrap();
        let root_c = root.canonicalize().unwrap();
        assert_eq!(out["path"], json!(root_c.join("in.txt").to_string_lossy()));
        assert_eq!(out["other"], json!("../x"), "only `path` keys are paths");
        assert_eq!(out["nested"][0]["path"], json!(root_c.join("sub/a.png").to_string_lossy()));
        assert!(p.confine(json!({"steps": [{"params": {"path": "../private/passwords.txt"}}]})).is_err());
        // A numeric `path` (a location in the document, not a file) is left alone.
        assert_eq!(p.confine(json!({"path": [0, 1]})).unwrap(), json!({"path": [0, 1]}));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn without_a_root_every_path_is_refused() {
        let p = PathPolicy { root: Err("none".into()) };
        assert!(p.check("a.docx").is_err());
        assert!(p.confine(json!({"text": "no paths"})).is_ok());
    }
}
