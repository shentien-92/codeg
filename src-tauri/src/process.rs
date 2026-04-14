use std::ffi::{OsStr, OsString};
use std::path::PathBuf;
use std::process::Command;

#[cfg(windows)]
use std::path::Path;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub fn configure_std_command(command: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
        set_utf8_env(command);
    }

    command
}

pub fn std_command<S>(program: S) -> Command
where
    S: AsRef<OsStr>,
{
    let mut command = Command::new(normalized_program(program));
    configure_std_command(&mut command);
    command
}

pub fn configure_tokio_command(
    command: &mut tokio::process::Command,
) -> &mut tokio::process::Command {
    #[cfg(windows)]
    {
        command.creation_flags(CREATE_NO_WINDOW);
        set_utf8_env(command);
    }

    command
}

/// Hint child processes to produce UTF-8 output on Windows.
///
/// Sets environment variables recognised by common runtimes (Python, MSYS2/Git
/// Bash, .NET console apps).  Not all programs honour these, but they cover the
/// most frequent sources of mojibake in practice.
#[cfg(windows)]
fn set_utf8_env<C: SetEnv>(command: &mut C) {
    // Python
    command.env("PYTHONUTF8", "1");
    command.env("PYTHONIOENCODING", "utf-8");
    // MSYS2 / Git-for-Windows / POSIX-layer tools
    command.env("LANG", "C.UTF-8");
    command.env("LC_ALL", "C.UTF-8");
}

/// Abstraction over the `.env()` method shared by std and tokio Command types.
#[cfg(windows)]
trait SetEnv {
    fn env(&mut self, key: &str, val: &str) -> &mut Self;
}

#[cfg(windows)]
impl SetEnv for Command {
    fn env(&mut self, key: &str, val: &str) -> &mut Self {
        Command::env(self, key, val)
    }
}

#[cfg(windows)]
impl SetEnv for tokio::process::Command {
    fn env(&mut self, key: &str, val: &str) -> &mut Self {
        tokio::process::Command::env(self, key, val)
    }
}

#[cfg(windows)]
fn maybe_windows_cmd_shim(program: &OsStr) -> Option<OsString> {
    let path = Path::new(program);
    if path.components().count() != 1 || path.extension().is_some() {
        return None;
    }

    let raw = program.to_string_lossy();
    let normalized = raw.to_ascii_lowercase();
    let needs_cmd_shim = matches!(
        normalized.as_str(),
        "npm" | "npx" | "pnpm" | "pnpx" | "yarn" | "yarnpkg" | "corepack"
    );

    if needs_cmd_shim {
        Some(OsString::from(format!("{raw}.cmd")))
    } else {
        None
    }
}

pub fn normalized_program<S>(program: S) -> OsString
where
    S: AsRef<OsStr>,
{
    #[cfg(windows)]
    {
        if let Some(shimmed) = maybe_windows_cmd_shim(program.as_ref()) {
            return shimmed;
        }
    }

    program.as_ref().to_os_string()
}

pub fn tokio_command<S>(program: S) -> tokio::process::Command
where
    S: AsRef<OsStr>,
{
    let mut command = tokio::process::Command::new(normalized_program(program));
    configure_tokio_command(&mut command);
    command
}

/// Detect Homebrew installation on macOS and ensure its `bin/` and `sbin/`
/// directories are in PATH.
///
/// On macOS, GUI-launched applications (Finder, Spotlight, Dock) inherit a
/// minimal PATH that typically does **not** include Homebrew's directories.
/// This means `which::which("node")`, `which::which("opencode")`, etc. will
/// fail even though the user has these tools installed via `brew install`.
///
/// Standard Homebrew prefixes:
/// - Apple Silicon: `/opt/homebrew`
/// - Intel:         `/usr/local`
///
/// If `HOMEBREW_PREFIX` is set it is used instead of the well-known defaults.
/// The function is a no-op on non-macOS platforms.
pub fn ensure_homebrew_in_path() {
    #[cfg(target_os = "macos")]
    {
        let current_path = std::env::var_os("PATH").unwrap_or_default();
        let current_str = current_path.to_string_lossy();

        // Collect Homebrew prefixes to check.
        let mut prefixes: Vec<PathBuf> = Vec::new();

        if let Ok(env_prefix) = std::env::var("HOMEBREW_PREFIX") {
            prefixes.push(PathBuf::from(env_prefix));
        }

        // Well-known default prefixes (Apple Silicon first, then Intel).
        prefixes.push(PathBuf::from("/opt/homebrew"));
        prefixes.push(PathBuf::from("/usr/local"));

        for prefix in prefixes {
            for subdir in &["bin", "sbin"] {
                let dir = prefix.join(subdir);
                if dir.is_dir() {
                    let dir_str = dir.to_string_lossy();
                    let sep = ":";
                    // Skip if already in PATH.
                    if current_str.split(sep).any(|p| p == dir_str.as_ref()) {
                        continue;
                    }
                    prepend_to_path(&dir);
                    eprintln!("[PATH] Homebrew: prepended {}", dir.display());
                }
            }
        }
    }
}

/// If `node` is not already in PATH, detect common Node.js version manager
/// installations (nvm, fnm, volta) and Homebrew, then prepend the best
/// matching bin directory to the process PATH so that **all** downstream
/// code (`which`, `Command`, child processes) can find node/npm/npx without
/// any special handling.
///
/// Call once at startup, after `fix_path_env::fix()` and
/// `ensure_homebrew_in_path()`.
#[cfg(feature = "tauri-runtime")]
pub fn ensure_node_in_path() {
    // Already reachable — nothing to do.
    if which::which("node").is_ok() {
        return;
    }

    let home = match dirs::home_dir() {
        Some(h) => h,
        None => return,
    };

    if let Some(bin_dir) = find_node_bin_dir(&home) {
        prepend_to_path(&bin_dir);
        eprintln!("[PATH] node not in PATH, prepended {}", bin_dir.display());
    }
}

/// Search common Node.js version manager directories and Homebrew for a
/// `node` binary and return the containing bin directory.
#[cfg(feature = "tauri-runtime")]
fn find_node_bin_dir(home: &std::path::Path) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();

    // ── nvm ──────────────────────────────────────────────────────────────
    let nvm_dir = std::env::var("NVM_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home.join(".nvm"));
    if nvm_dir.is_dir() {
        let versions_dir = nvm_dir.join("versions").join("node");

        // Prefer the version pointed to by the "default" alias.
        let default_alias = nvm_dir.join("alias").join("default");
        if let Ok(alias) = std::fs::read_to_string(&default_alias) {
            let alias = alias.trim().to_string();
            if let Ok(entries) = std::fs::read_dir(&versions_dir) {
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    let stripped = name.trim_start_matches('v');
                    if stripped.starts_with(&alias) || name.starts_with(&alias) {
                        candidates.push(entry.path().join("bin"));
                    }
                }
            }
        }

        // Fall back: all installed versions, newest first.
        if let Ok(mut entries) = std::fs::read_dir(&versions_dir)
            .map(|rd| rd.flatten().map(|e| e.path()).collect::<Vec<_>>())
        {
            entries.sort();
            entries.reverse();
            for entry in entries {
                candidates.push(entry.join("bin"));
            }
        }
    }

    // ── fnm ──────────────────────────────────────────────────────────────
    let fnm_dir = std::env::var("FNM_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home.join(".local").join("share").join("fnm"));
    let fnm_versions = fnm_dir.join("node-versions");
    if fnm_versions.is_dir() {
        if let Ok(mut entries) = std::fs::read_dir(&fnm_versions)
            .map(|rd| rd.flatten().map(|e| e.path()).collect::<Vec<_>>())
        {
            entries.sort();
            entries.reverse();
            for entry in entries {
                candidates.push(entry.join("installation").join("bin"));
            }
        }
    }

    // ── volta ────────────────────────────────────────────────────────────
    let volta_home = std::env::var("VOLTA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home.join(".volta"));
    let volta_bin = volta_home.join("bin");
    if volta_bin.is_dir() {
        candidates.push(volta_bin);
    }

    // ── Homebrew (macOS) ─────────────────────────────────────────────────
    // Homebrew-installed Node.js lives under <prefix>/opt/node@<ver>/bin
    // or <prefix>/bin (symlinked). Check the opt-linked paths first for a
    // more specific match, then fall back to the main bin directory.
    #[cfg(target_os = "macos")]
    {
        let brew_prefixes: Vec<PathBuf> = std::env::var("HOMEBREW_PREFIX")
            .map(|p| vec![PathBuf::from(p)])
            .unwrap_or_else(|_| {
                vec![
                    PathBuf::from("/opt/homebrew"),
                    PathBuf::from("/usr/local"),
                ]
            });

        for prefix in &brew_prefixes {
            // Check versioned opt links (e.g. /opt/homebrew/opt/node@22/bin)
            let opt_dir = prefix.join("opt");
            if opt_dir.is_dir() {
                if let Ok(entries) = std::fs::read_dir(&opt_dir) {
                    let mut node_opts: Vec<PathBuf> = entries
                        .flatten()
                        .filter(|e| {
                            let name = e.file_name().to_string_lossy().to_string();
                            name == "node" || name.starts_with("node@")
                        })
                        .map(|e| e.path().join("bin"))
                        .collect();
                    // Sort so that unversioned "node" comes first, then
                    // versioned entries in descending order.
                    node_opts.sort();
                    node_opts.reverse();
                    candidates.extend(node_opts);
                }
            }

            // Fall back to the main Homebrew bin directory where `node`
            // may be symlinked.
            let bin_dir = prefix.join("bin");
            if bin_dir.is_dir() {
                candidates.push(bin_dir);
            }
        }
    }

    // Return the first candidate that actually contains a `node` binary.
    candidates.into_iter().find(|dir| dir.join("node").is_file())
}

/// Prepend a directory to the process `PATH` environment variable.
pub(crate) fn prepend_to_path(dir: &std::path::Path) {
    let sep = if cfg!(windows) { ";" } else { ":" };
    let current = std::env::var_os("PATH").unwrap_or_default();
    let mut new_path = OsString::from(dir);
    new_path.push(sep);
    new_path.push(current);
    std::env::set_var("PATH", new_path);
}

/// Return the user-local npm prefix directory (`~/.codeg/npm-global/`).
///
/// Used as a fallback when `npm install -g` fails with EACCES because the
/// system global prefix (e.g. `/usr/local/lib/node_modules/`) is not writable.
pub(crate) fn user_npm_prefix() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".codeg").join("npm-global"))
}

/// Ensure the user-local npm prefix `bin/` directory is in `PATH` so that
/// binaries installed via the EACCES fallback can be found by `which` and
/// child processes.  Safe to call even if the directory does not exist yet.
///
/// On Unix, `npm install -g --prefix=<p>` places binaries in `<p>/bin/`.
/// On Windows, binaries are placed directly in `<p>/`.
pub fn ensure_user_npm_prefix_in_path() {
    if let Some(prefix) = user_npm_prefix() {
        let bin_dir = if cfg!(windows) {
            prefix
        } else {
            prefix.join("bin")
        };
        // Avoid adding duplicates.
        let current = std::env::var_os("PATH").unwrap_or_default();
        let bin_str = bin_dir.to_string_lossy();
        let sep = if cfg!(windows) { ";" } else { ":" };
        if !current
            .to_string_lossy()
            .split(sep)
            .any(|p| p == bin_str.as_ref())
        {
            prepend_to_path(&bin_dir);
        }
    }
}
