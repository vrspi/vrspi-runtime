use std::ffi::OsString;

/// Product name for prose in the UI and user-facing messages.
///
/// Separate from [`crate::EXECUTABLE_NAME`], which is the lowercase command
/// users type. Neither may be used for compatibility contracts: config
/// directories, the `delivery = "herdr"` config value, `herdr-plugin.toml`,
/// API error codes, and package-manager names all stay as they are.
pub(crate) const PRODUCT_NAME: &str = "Vrspi";

/// Whether the self-updater runs.
///
/// This was off while the endpoints below still pointed at upstream Herdr:
/// updating would have downloaded a Herdr release and silently replaced the
/// running Vrspi with it. Now that they point at Vrspi's own manifest, the
/// updater installs Vrspi releases and the surfaces are on.
pub(crate) const SELF_UPDATES_ENABLED: bool = true;

/// Whether a release may open a product announcement at startup.
///
/// Off, and separately from updates on purpose. Vrspi's manifest publishes no
/// announcement field, so the only announcements that can be on disk are
/// upstream Herdr's, saved while this machine ran Herdr. A stale one would
/// seize the screen at startup to advertise a product this is not.
pub(crate) const PRODUCT_ANNOUNCEMENTS_ENABLED: bool = false;

/// Where `vrspi update` and the installer look for the published release.
///
/// Both read the same manifest on purpose, so an install and an update can
/// never disagree about what the latest release is. Hosted on vrspi.com so the
/// source repository can be private. The cost is one VPS behind it: if
/// vrspi.com is down, nobody can install *or* update until it is back.
pub(crate) const UPDATE_MANIFEST_URL: &str = "https://vrspi.com/runtime/latest.json";

/// Returns a branded environment override, falling back to its Herdr-era
/// spelling so existing sessions, integrations, and automation keep working.
/// When both are present, the Vrspi spelling wins.
pub(crate) fn env_var(primary: &str, legacy: &str) -> Result<String, std::env::VarError> {
    match std::env::var(primary) {
        Err(std::env::VarError::NotPresent) => std::env::var(legacy),
        result => result,
    }
}

/// Clears a branded override in both spellings.
///
/// Reads fall back to the legacy name, so clearing only the Vrspi spelling
/// leaves a stale `HERDR_*` value visible. That is invisible in CI, where
/// neither is set, and bites anyone running inside a managed pane.
pub(crate) fn remove_env_var(primary: &str, legacy: &str) {
    std::env::remove_var(primary);
    std::env::remove_var(legacy);
}

pub(crate) fn env_var_os(primary: &str, legacy: &str) -> Option<OsString> {
    std::env::var_os(primary).or_else(|| std::env::var_os(legacy))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn removing_a_branded_override_clears_the_legacy_spelling_too() {
        let _guard = env_lock().lock().unwrap_or_else(|err| err.into_inner());
        let primary = "VRSPI_TEST_BRAND_REMOVE";
        let legacy = "HERDR_TEST_BRAND_REMOVE";
        std::env::set_var(primary, "vrspi");
        std::env::set_var(legacy, "legacy");
        remove_env_var(primary, legacy);
        assert!(env_var(primary, legacy).is_err());
    }

    #[test]
    fn branded_environment_wins_with_legacy_fallback() {
        let _guard = env_lock().lock().unwrap_or_else(|err| err.into_inner());
        let primary = "VRSPI_TEST_BRAND_ENV";
        let legacy = "HERDR_TEST_BRAND_ENV";
        std::env::remove_var(primary);
        std::env::set_var(legacy, "legacy");
        assert_eq!(env_var(primary, legacy).as_deref(), Ok("legacy"));
        std::env::set_var(primary, "vrspi");
        assert_eq!(env_var(primary, legacy).as_deref(), Ok("vrspi"));
        std::env::remove_var(primary);
        std::env::remove_var(legacy);
    }
}
