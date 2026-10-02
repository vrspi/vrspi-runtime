use std::fs;
use std::io;
use std::path::Path;

pub(crate) fn remove_file_if_exists(path: &Path) -> io::Result<bool> {
    match fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(err),
    }
}

/// Hook files a previous install left behind that this one supersedes.
///
/// Two kinds. The Herdr-era names, because the assets were renamed to
/// `vrspi-agent-state.*` and the old files are still live: the server exports
/// both the VRSPI_ and HERDR_ environment variables, so an orphaned
/// `herdr-agent-state.sh` keeps firing and reports the same pane twice. And on
/// Windows the sibling shell hook, which the PowerShell hook replaces.
///
/// Only files carrying an integration marker are removed, so a hand-written
/// hook that happens to share the name is never deleted.
fn superseded_hook_names(hook_path: &Path) -> Vec<String> {
    let mut names: Vec<String> = ["sh", "ps1", "ts", "js"]
        .iter()
        .map(|extension| format!("herdr-agent-state.{extension}"))
        .collect();
    if cfg!(windows) {
        if let Some(current) = hook_path.file_name().and_then(|name| name.to_str()) {
            if current != "vrspi-agent-state.sh" {
                names.push("vrspi-agent-state.sh".to_string());
            }
        }
    }
    names
}

/// Removes hook files this install supersedes. Returns whether any went away.
pub(crate) fn remove_superseded_hook_files(hook_path: &Path) -> io::Result<bool> {
    let mut removed = false;
    for name in superseded_hook_names(hook_path) {
        let candidate = hook_path.with_file_name(&name);
        if candidate == hook_path {
            continue;
        }
        let content = match fs::read_to_string(&candidate) {
            Ok(content) => content,
            Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
            // A hook we cannot read is a hook we must not delete.
            Err(_) => continue,
        };
        if content.contains("HERDR_INTEGRATION_ID=") || content.contains("VRSPI_INTEGRATION_ID=") {
            fs::remove_file(&candidate)?;
            removed = true;
        }
    }
    Ok(removed)
}

pub(crate) fn remove_dir_all_if_exists(path: &Path) -> io::Result<bool> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(true),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(err),
    }
}

pub(crate) fn make_executable(_path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mut perms = fs::metadata(_path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(_path, perms)?;
    }

    Ok(())
}
