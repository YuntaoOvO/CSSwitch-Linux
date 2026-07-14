use std::fs;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};

use csswitch_config;

/// Platform `O_NOFOLLOW` without libc. macOS/BSD=0x0100, Linux=0x20000.
const fn libc_o_nofollow() -> i32 {
    if cfg!(target_os = "linux") {
        0x2_0000
    } else {
        0x0100
    }
}

const OPERATION_LOG_MAX_BYTES: u64 = 1_048_576; // 1 MiB

pub fn log_path(name: &str) -> PathBuf {
    csswitch_config::default_dir().join("logs").join(name)
}

pub fn open_log(name: &str) -> std::io::Result<fs::File> {
    let p = log_path(name);
    if let Some(parent) = p.parent() {
        csswitch_config::assert_not_symlink(parent)?;
        fs::create_dir_all(parent)?;
        let _ = fs::set_permissions(parent, fs::Permissions::from_mode(0o700));
    }
    csswitch_config::assert_not_symlink(&p)?;
    let f = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .custom_flags(libc_o_nofollow())
        .open(&p)?;
    let _ = fs::set_permissions(&p, fs::Permissions::from_mode(0o600));
    Ok(f)
}

pub fn append_operation_log(line: &str) {
    let p = log_path("operation.log");
    let Some(parent) = p.parent() else { return };
    if csswitch_config::assert_not_symlink(parent).is_err()
        || csswitch_config::assert_not_symlink(&p).is_err()
    {
        return;
    }
    if fs::create_dir_all(parent).is_err() {
        return;
    }
    let _ = fs::set_permissions(parent, fs::Permissions::from_mode(0o700));
    rotate_operation_log_if_needed(&p, line.len() as u64 + 1);
    let mut f = match fs::OpenOptions::new()
        .append(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc_o_nofollow())
        .open(&p)
    {
        Ok(f) => f,
        Err(_) => return,
    };
    let _ = fs::set_permissions(&p, fs::Permissions::from_mode(0o600));
    let _ = writeln!(f, "{line}");
}

fn operation_log_archive_path(p: &Path) -> PathBuf {
    p.with_file_name("operation.log.1")
}

fn should_rotate_operation_log(current_bytes: u64, incoming_bytes: u64) -> bool {
    current_bytes.saturating_add(incoming_bytes) > OPERATION_LOG_MAX_BYTES
}

fn rotate_operation_log_if_needed(p: &Path, incoming_bytes: u64) {
    let Ok(md) = fs::metadata(p) else { return };
    if !should_rotate_operation_log(md.len(), incoming_bytes) {
        return;
    }
    let archive = operation_log_archive_path(p);
    if csswitch_config::assert_not_symlink(&archive).is_err() {
        return;
    }
    let _ = fs::remove_file(&archive);
    if fs::rename(p, &archive).is_ok() {
        let _ = fs::set_permissions(&archive, fs::Permissions::from_mode(0o600));
    }
}

pub fn redact(s: &str, secret: &str) -> String {
    if secret.is_empty() {
        s.to_string()
    } else {
        s.replace(secret, "****")
    }
}

pub fn tail_file(path: &Path, max: usize) -> String {
    match fs::read(path) {
        Ok(b) => {
            let start = b.len().saturating_sub(max);
            String::from_utf8_lossy(&b[start..]).trim().to_string()
        }
        Err(_) => String::new(),
    }
}

pub fn kill_child(slot: &mut Option<Child>) {
    if let Some(mut c) = slot.take() {
        let _ = c.kill();
        let _ = c.wait();
    }
}

/// Open a URL with the system browser.
pub fn open_in_browser(url: &str) -> Result<(), String> {
    let cmd = if cfg!(target_os = "linux") {
        // Try xdg-open first, then fallback to sensible-browser
        if Command::new("xdg-open").arg("--version").output().is_ok() {
            "xdg-open"
        } else {
            "sensible-browser"
        }
    } else {
        "open" // macOS
    };
    let st = Command::new(cmd)
        .arg(url)
        .status()
        .map_err(|e| format!("打开浏览器失败：{e}"))?;
    if !st.success() {
        return Err(format!("{} 非零退出（{:?}）", cmd, st.code()));
    }
    Ok(())
}

/// Find the repository root (for development asset discovery).
pub fn repo_root() -> Option<PathBuf> {
    let marker = ".git";
    let mut current = std::env::current_exe().ok()?;
    current.pop(); // remove binary name
    for _ in 0..8 {
        if current.join(marker).is_dir() {
            return Some(current);
        }
        if !current.pop() {
            break;
        }
    }
    // Fallback: check CWD
    let cwd = std::env::current_dir().ok()?;
    let mut p = cwd;
    for _ in 0..8 {
        if p.join(marker).is_dir() {
            return Some(p);
        }
        if !p.pop() {
            break;
        }
    }
    None
}

/// Search PATH for an executable binary.
pub fn find_in_path(name: &str) -> Option<PathBuf> {
    if let Ok(paths) = std::env::var("PATH") {
        for dir in paths.split(':') {
            let candidate = PathBuf::from(dir).join(name);
            if candidate.is_file() {
                if let Ok(meta) = std::fs::metadata(&candidate) {
                    use std::os::unix::fs::PermissionsExt;
                    if meta.permissions().mode() & 0o111 != 0 {
                        return Some(candidate);
                    }
                }
            }
        }
    }
    None
}

pub fn sandbox_home() -> PathBuf {
    csswitch_config::default_dir().join("sandbox").join("home")
}

pub fn daemon_pid_file() -> PathBuf {
    csswitch_config::default_dir().join("daemon.pid")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redact_replaces_nonempty_secret_only() {
        assert_eq!(redact("abc secret abc", "secret"), "abc **** abc");
        assert_eq!(redact("abc", ""), "abc");
    }

    #[test]
    fn operation_log_rotation_threshold() {
        assert!(!should_rotate_operation_log(1_048_575, 1));
        assert!(should_rotate_operation_log(1_048_575, 2));
    }

    #[test]
    fn operation_log_archive_is_single_sibling_file() {
        assert_eq!(
            operation_log_archive_path(Path::new("/tmp/operation.log")),
            Path::new("/tmp/operation.log.1")
        );
    }

    #[test]
    fn sandbox_home_is_under_config_dir() {
        let h = sandbox_home();
        assert!(h.to_string_lossy().contains(".csswitch"));
        assert!(h.to_string_lossy().contains("sandbox"));
    }
}
