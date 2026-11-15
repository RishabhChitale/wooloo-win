//! Finds the Herdr executable without trusting the process PATH.
//!
//! A GUI app started from the Start menu inherits Explorer's environment, which lacks the PATH entry
//! Herdr's per-user installer wrote to the registry. So on Windows we read `Path` fresh from the registry,
//! probe the installer's own directories, then ask `where.exe`. The result is an absolute path that every
//! caller uses instead of the bare name `herdr`.
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// A resolved Herdr executable plus everything that was looked at on the way.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolved {
    pub path: PathBuf,
    pub tried: Vec<String>,
}

/// Splits a registry `Path` value: `;`-separated, trims blanks, strips surrounding quotes, drops empties.
pub fn parse_path_value(raw: &str) -> Vec<String> {
    raw.split(';')
        .map(|e| e.trim().trim_matches('"').trim().to_string())
        .filter(|e| !e.is_empty())
        .collect()
}

/// `ExpandEnvironmentStrings` semantics: `%NAME%` is replaced via `lookup` (case-insensitive on Windows is
/// the lookup's job); an unknown variable, or a lone `%`, is left as written.
pub fn expand_env(s: &str, lookup: &dyn Fn(&str) -> Option<String>) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('%') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        match after.find('%') {
            Some(j) if j > 0 => match lookup(&after[..j]) {
                Some(v) => {
                    out.push_str(&v);
                    rest = &after[j + 1..];
                }
                None => {
                    out.push('%');
                    rest = after;
                }
            },
            _ => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Directories Herdr's Windows installer uses (install.ps1): the visible bin dir, the standalone `current`
/// junction under HERDR_HOME (default `%USERPROFILE%\.herdr`), and HERDR_INSTALL_DIR when set.
pub fn installer_dirs(lookup: &dyn Fn(&str) -> Option<String>) -> Vec<String> {
    let mut v = Vec::new();
    if let Some(d) = lookup("HERDR_INSTALL_DIR").filter(|d| !d.is_empty()) {
        v.push(d);
    }
    if let Some(l) = lookup("LOCALAPPDATA").filter(|d| !d.is_empty()) {
        v.push(format!("{l}\\Programs\\Herdr\\bin"));
    }
    let home = lookup("HERDR_HOME").filter(|d| !d.is_empty()).or_else(|| lookup("USERPROFILE").filter(|d| !d.is_empty()).map(|u| format!("{u}\\.herdr")));
    if let Some(h) = home {
        v.push(format!("{h}\\packages\\standalone\\current"));
    }
    v
}

const NAMES: [&str; 2] = ["herdr.exe", "herdr"];

/// Looks for `herdr.exe` / `herdr` in each dir (in order). Every probe is appended to `tried`.
pub fn find_in_dirs(dirs: &[String], label: &str, exists: &dyn Fn(&Path) -> bool, tried: &mut Vec<String>) -> Option<PathBuf> {
    for d in dirs {
        let mut hit = None;
        for n in NAMES {
            let p = Path::new(d).join(n);
            if exists(&p) {
                hit = Some(p);
                break;
            }
        }
        match hit {
            Some(p) => {
                tried.push(format!("{label} {d}: found {}", p.display()));
                return Some(p);
            }
            None => tried.push(format!("{label} {d}: no herdr.exe or herdr")),
        }
    }
    None
}

fn env_lookup(k: &str) -> Option<String> {
    std::env::var(k).ok()
}

/// Reads one registry `Path` value (unexpanded). None on non-Windows or when absent.
#[cfg(windows)]
fn registry_path(hive: &str) -> Result<String, String> {
    use winreg::{enums::*, RegKey};
    let (root, sub) = if hive == "HKCU" {
        (RegKey::predef(HKEY_CURRENT_USER), "Environment")
    } else {
        (RegKey::predef(HKEY_LOCAL_MACHINE), "SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment")
    };
    let k = root.open_subkey(sub).map_err(|e| format!("cannot open {hive}\\{sub}: {e}"))?;
    k.get_value::<String, _>("Path").map_err(|e| format!("cannot read {hive}\\{sub}\\Path: {e}"))
}
#[cfg(not(windows))]
fn registry_path(_hive: &str) -> Result<String, String> {
    Err("registry is Windows-only".into())
}

fn where_exe() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let o = std::process::Command::new("where.exe").arg("herdr.exe").creation_flags(0x0800_0000).output().ok()?;
        if !o.status.success() {
            return None;
        }
        let t = String::from_utf8_lossy(&o.stdout);
        t.lines().map(|l| l.trim()).find(|l| !l.is_empty()).map(PathBuf::from)
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// Full resolution, uncached. Err carries the list of everything tried, one line each.
pub fn resolve_uncached() -> Result<Resolved, String> {
    let mut tried = Vec::new();
    let exists = |p: &Path| p.is_file();
    if cfg!(windows) {
        for hive in ["HKCU", "HKLM"] {
            match registry_path(hive) {
                Ok(raw) => {
                    let dirs: Vec<String> = parse_path_value(&raw).iter().map(|e| expand_env(e, &env_lookup)).collect();
                    if let Some(p) = find_in_dirs(&dirs, &format!("{hive} Path entry"), &exists, &mut tried) {
                        return Ok(Resolved { path: p, tried });
                    }
                }
                Err(e) => tried.push(e),
            }
        }
    }
    let inst = installer_dirs(&env_lookup);
    if cfg!(windows) {
        if let Some(p) = find_in_dirs(&inst, "installer dir", &exists, &mut tried) {
            return Ok(Resolved { path: p, tried });
        }
        match where_exe() {
            Some(p) => {
                tried.push(format!("where.exe herdr.exe: {}", p.display()));
                return Ok(Resolved { path: p, tried });
            }
            None => tried.push("where.exe herdr.exe: not found".into()),
        }
    }
    // Process PATH last (the only source on macOS/Linux).
    let dirs: Vec<String> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).map(|d| d.to_string_lossy().into_owned()).collect()).unwrap_or_default();
    let names: &[&str] = if cfg!(windows) { &NAMES } else { &["herdr"] };
    for d in &dirs {
        for n in names {
            let p = Path::new(d).join(n);
            if exists(&p) {
                tried.push(format!("process PATH {d}: found {}", p.display()));
                return Ok(Resolved { path: p, tried });
            }
        }
    }
    tried.push(format!("process PATH ({} entries): no herdr", dirs.len()));
    Err(format!("herdr not found. Tried:\n  {}", tried.join("\n  ")))
}

static CACHE: Mutex<Option<Resolved>> = Mutex::new(None);

/// Cached resolution. A cached path that no longer exists is re-resolved; failures are not cached.
pub fn resolve_herdr() -> Result<Resolved, String> {
    let mut c = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(r) = c.as_ref() {
        if r.path.is_file() {
            return Ok(r.clone());
        }
    }
    let r = resolve_uncached()?;
    *c = Some(r.clone());
    Ok(r)
}

/// Drops the cache so the next call looks again (call after a spawn failure).
pub fn invalidate() {
    *CACHE.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// Program to spawn: the absolute path, or the bare name when nothing was found (the spawn then fails
/// with the OS error; use `resolve_herdr()` for the full "tried" text).
pub fn herdr_program() -> PathBuf {
    resolve_herdr().map(|r| r.path).unwrap_or_else(|_| PathBuf::from("herdr"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(k: &str) -> Option<String> {
        match k.to_ascii_uppercase().as_str() {
            "USERPROFILE" => Some("C:\\Users\\Ann".into()),
            "SYSTEMROOT" => Some("C:\\Windows".into()),
            "LOCALAPPDATA" => Some("C:\\Users\\Ann\\AppData\\Local".into()),
            _ => None,
        }
    }

    #[test]
    fn parses_quotes_semicolons_blanks() {
        let v = parse_path_value("C:\\a;\"C:\\Program Files\\b\";; C:\\c ;");
        assert_eq!(v, vec!["C:\\a", "C:\\Program Files\\b", "C:\\c"]);
        assert!(parse_path_value("").is_empty());
        assert!(parse_path_value(";;").is_empty());
    }

    #[test]
    fn expands_vars() {
        assert_eq!(expand_env("%USERPROFILE%\\bin", &env), "C:\\Users\\Ann\\bin");
        assert_eq!(expand_env("%SystemRoot%\\system32;%SYSTEMROOT%", &env), "C:\\Windows\\system32;C:\\Windows");
    }

    #[test]
    fn unknown_and_lone_percent_left_alone() {
        assert_eq!(expand_env("%NOPE%\\x", &env), "%NOPE%\\x");
        assert_eq!(expand_env("100%", &env), "100%");
        assert_eq!(expand_env("a%%b", &env), "a%%b");
        assert_eq!(expand_env("%USERPROFILE", &env), "%USERPROFILE");
    }

    #[test]
    fn sample_registry_value_end_to_end() {
        let raw = "%SystemRoot%\\system32;\"%LOCALAPPDATA%\\Programs\\Herdr\\bin\";%USERPROFILE%\\.cargo\\bin;";
        let dirs: Vec<String> = parse_path_value(raw).iter().map(|e| expand_env(e, &env)).collect();
        assert_eq!(dirs, vec!["C:\\Windows\\system32", "C:\\Users\\Ann\\AppData\\Local\\Programs\\Herdr\\bin", "C:\\Users\\Ann\\.cargo\\bin"]);
    }

    #[test]
    fn installer_dirs_default_and_overrides() {
        let d = installer_dirs(&env);
        assert_eq!(d, vec!["C:\\Users\\Ann\\AppData\\Local\\Programs\\Herdr\\bin", "C:\\Users\\Ann\\.herdr\\packages\\standalone\\current"]);
        let with = |k: &str| match k {
            "HERDR_HOME" => Some("D:\\h".into()),
            "HERDR_INSTALL_DIR" => Some("D:\\bin".into()),
            _ => env(k),
        };
        let d = installer_dirs(&with);
        assert_eq!(d[0], "D:\\bin");
        assert_eq!(d.last().unwrap(), "D:\\h\\packages\\standalone\\current");
    }

    #[test]
    fn find_in_dirs_prefers_exe_and_records_every_probe() {
        let dirs = vec!["C:\\x".to_string(), "C:\\y".to_string()];
        let mut tried = Vec::new();
        let ex = |p: &Path| p.ends_with("herdr") && p.to_string_lossy().contains("y");
        let hit = find_in_dirs(&dirs, "dir", &ex, &mut tried).unwrap();
        assert!(hit.to_string_lossy().contains('y'));
        assert_eq!(tried.len(), 2);
        assert!(tried[0].contains("C:\\x") && tried[0].contains("no herdr"));
        assert!(tried[1].contains("found"));
        let mut t2 = Vec::new();
        assert!(find_in_dirs(&dirs, "dir", &|_| false, &mut t2).is_none());
        assert_eq!(t2.len(), 2);
    }
}
