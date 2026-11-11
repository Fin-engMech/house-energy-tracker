//! Where the app keeps things on each system.
//!
//! | | Linux | macOS | Windows |
//! |---|---|---|---|
//! | data | `$XDG_DATA_HOME/house-energy-tracker` (`~/.local/share/…`) | `~/Library/Application Support/House Energy Tracker` | `%APPDATA%\House Energy Tracker` |
//! | cache | `$XDG_CACHE_HOME/house-energy-tracker` (`~/.cache/…`) | `~/Library/Caches/House Energy Tracker` | `%LOCALAPPDATA%\House Energy Tracker\Cache` |
//! | exports | the XDG Downloads folder | `~/Downloads` | `%USERPROFILE%\Downloads` |

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::{APP_ID, APP_NAME};

pub fn home() -> PathBuf {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn env_dir(var: &str) -> Option<PathBuf> {
    std::env::var_os(var).map(PathBuf::from).filter(|p| p.is_absolute())
}

pub fn data_dir() -> PathBuf {
    if cfg!(windows) {
        env_dir("APPDATA").unwrap_or_else(|| home().join("AppData/Roaming")).join(APP_NAME)
    } else if cfg!(target_os = "macos") {
        home().join("Library/Application Support").join(APP_NAME)
    } else {
        env_dir("XDG_DATA_HOME").unwrap_or_else(|| home().join(".local/share")).join(APP_ID)
    }
}

pub fn cache_dir() -> PathBuf {
    if cfg!(windows) {
        env_dir("LOCALAPPDATA").unwrap_or_else(|| home().join("AppData/Local")).join(APP_NAME).join("Cache")
    } else if cfg!(target_os = "macos") {
        home().join("Library/Caches").join(APP_NAME)
    } else {
        env_dir("XDG_CACHE_HOME").unwrap_or_else(|| home().join(".cache")).join(APP_ID)
    }
}

/// `canonicalize`, but without the `\\?\` prefix it adds on Windows, which Explorer doesn't accept
/// and which looks odd in the interface.
pub fn canonical(p: &Path) -> std::io::Result<PathBuf> {
    p.canonicalize().map(plain)
}

/// `\\?\C:\x` becomes `C:\x` and `\\?\UNC\server\share` becomes `\\server\share`. Paths that need the
/// prefix (too long for the old limit) are left alone.
fn plain(p: PathBuf) -> PathBuf {
    let Some(s) = p.to_str() else { return p };
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    match s.strip_prefix(r"\\?\") {
        Some(rest) if rest.len() < 260 && rest.as_bytes().get(1) == Some(&b':') => PathBuf::from(rest),
        _ => p,
    }
}

/// Read `XDG_DOWNLOAD_DIR` from the contents of `~/.config/user-dirs.dirs`.
fn xdg_download_dir(user_dirs: &str, home: &Path) -> Option<PathBuf> {
    let line = user_dirs.lines().map(str::trim).find(|l| l.starts_with("XDG_DOWNLOAD_DIR="))?;
    let v = line["XDG_DOWNLOAD_DIR=".len()..].trim_matches('"');
    let p = match v.strip_prefix("$HOME") {
        Some(rest) => home.join(rest.trim_start_matches('/')),
        None => PathBuf::from(v),
    };
    (p.is_absolute() && p != home).then_some(p)
}

pub fn downloads_dir() -> PathBuf {
    let home = home();
    if cfg!(all(unix, not(target_os = "macos"))) {
        let cfg = env_dir("XDG_CONFIG_HOME").unwrap_or_else(|| home.join(".config"));
        if let Some(p) = std::fs::read_to_string(cfg.join("user-dirs.dirs")).ok().and_then(|t| xdg_download_dir(&t, &home)) {
            return p;
        }
    }
    home.join("Downloads")
}

/// Append a line to the log file in the cache folder (and print it). With no console window on
/// Windows, and none when launched from a menu anywhere, this is where errors end up.
pub fn log(msg: &str) {
    eprintln!("{msg}");
    // Tests print only: they must never write into the real cache folder.
    if cfg!(test) {
        return;
    }
    let dir = cache_dir();
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("energy-tracker.log")) {
        let _ = writeln!(f, "{} {msg}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"));
    }
}

/// Start each run with a fresh log, keeping the previous one.
pub fn rotate_log() {
    let dir = cache_dir();
    let log = dir.join("energy-tracker.log");
    if log.exists() {
        let _ = std::fs::rename(&log, dir.join("energy-tracker.log.old"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_the_windows_verbatim_prefix() {
        assert_eq!(plain(PathBuf::from(r"\\?\C:\Users\sam\data.json")), PathBuf::from(r"C:\Users\sam\data.json"));
        assert_eq!(plain(PathBuf::from(r"\\?\UNC\nas\home\data.json")), PathBuf::from(r"\\nas\home\data.json"));
        assert_eq!(plain(PathBuf::from(r"\\?\Volume{1234}\data.json")), PathBuf::from(r"\\?\Volume{1234}\data.json"));
        assert_eq!(plain(PathBuf::from("/home/sam/data.json")), PathBuf::from("/home/sam/data.json"));
    }

    // The XDG file is only read on Linux, and "/home/sam" isn't an absolute path on Windows.
    #[cfg(unix)]
    #[test]
    fn reads_xdg_download_dir() {
        let home = Path::new("/home/sam");
        let t = "# comment\nXDG_DESKTOP_DIR=\"$HOME/Desktop\"\nXDG_DOWNLOAD_DIR=\"$HOME/Téléchargements\"\n";
        assert_eq!(xdg_download_dir(t, home), Some(PathBuf::from("/home/sam/Téléchargements")));
        assert_eq!(xdg_download_dir("XDG_DOWNLOAD_DIR=\"/data/dl\"", home), Some(PathBuf::from("/data/dl")));
        // pointing it at the home folder itself means "not set"
        assert_eq!(xdg_download_dir("XDG_DOWNLOAD_DIR=\"$HOME/\"", home), None);
        assert_eq!(xdg_download_dir("", home), None);
    }
}
