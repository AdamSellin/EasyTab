//! Langue des messages affichés à l'utilisateur : anglais par défaut, français
//! si le système est en français. `EASYTAB_LANG` (`fr` ou `en`) force le choix.
//!
//! Partagé avec easytab-cli par `#[path]`, comme `config.rs` : le module doit y
//! être déclaré à la racine sous le nom `lang` (la macro [`tr!`] y renvoie).

use std::sync::OnceLock;

/// Vrai si les messages doivent être en français. Lu une seule fois.
pub fn fr() -> bool {
    static FR: OnceLock<bool> = OnceLock::new();
    *FR.get_or_init(|| detect(|name| std::env::var(name).ok(), system_is_french))
}

/// Choisit le texte selon la langue : `tr("Settings", "Réglages")`.
pub fn tr(en: &'static str, fr: &'static str) -> &'static str {
    if self::fr() {
        fr
    } else {
        en
    }
}

/// Comme [`tr`], avec des chaînes de format : `tr!("Installed in {path}",
/// "Installé dans {path}")` renvoie une `String`.
#[macro_export]
macro_rules! tr {
    ($en:literal, $fr:literal $(,)?) => {
        if $crate::lang::fr() {
            format!($fr)
        } else {
            format!($en)
        }
    };
}

/// `EASYTAB_LANG` d'abord, puis la première variable de locale définie
/// (`LC_ALL`, `LC_MESSAGES`, `LANG`, dans l'ordre de POSIX), sinon la langue
/// du système.
fn detect(var: impl Fn(&str) -> Option<String>, system: impl FnOnce() -> bool) -> bool {
    for name in ["EASYTAB_LANG", "LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Some(value) = var(name).filter(|value| !value.trim().is_empty()) {
            return is_french(&value);
        }
    }
    system()
}

/// `fr`, `fr_FR.UTF-8`, `fr-CA`…
fn is_french(locale: &str) -> bool {
    locale.trim().to_ascii_lowercase().starts_with("fr")
}

/// Langue de l'interface de Windows.
#[cfg(windows)]
fn system_is_french() -> bool {
    use windows_sys::Win32::Globalization::GetUserDefaultUILanguage;
    const LANG_FRENCH: u16 = 0x0c;
    // SAFETY: appel Win32 sans argument.
    let language = unsafe { GetUserDefaultUILanguage() };
    language & 0x3ff == LANG_FRENCH
}

/// Langue de macOS, quand le terminal ne pose pas `LANG`.
#[cfg(target_os = "macos")]
fn system_is_french() -> bool {
    std::process::Command::new("defaults")
        .args(["read", "-g", "AppleLocale"])
        .stderr(std::process::Stdio::null())
        .output()
        .is_ok_and(|output| is_french(&String::from_utf8_lossy(&output.stdout)))
}

#[cfg(not(any(windows, target_os = "macos")))]
fn system_is_french() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detect_with(vars: &[(&str, &str)], system: bool) -> bool {
        detect(
            |name| {
                vars.iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| value.to_string())
            },
            || system,
        )
    }

    #[test]
    fn english_by_default() {
        assert!(!detect_with(&[], false));
        assert!(!detect_with(&[("LANG", "en_US.UTF-8")], false));
        assert!(!detect_with(&[("LANG", "C")], true));
    }

    #[test]
    fn french_locale() {
        assert!(detect_with(&[("LANG", "fr_FR.UTF-8")], false));
        assert!(detect_with(&[("LC_MESSAGES", "fr_CA")], false));
        assert!(detect_with(&[("LC_ALL", "fr")], false));
    }

    #[test]
    fn posix_order() {
        assert!(!detect_with(
            &[("LC_ALL", "en_GB.UTF-8"), ("LANG", "fr_FR.UTF-8")],
            false
        ));
        assert!(detect_with(
            &[("LC_MESSAGES", "fr_FR.UTF-8"), ("LANG", "en_US.UTF-8")],
            false
        ));
        // Une variable vide est ignorée.
        assert!(detect_with(&[("LC_ALL", ""), ("LANG", "fr_BE")], false));
    }

    #[test]
    fn system_language_without_locale() {
        assert!(detect_with(&[], true));
        assert!(detect_with(&[("LANG", "")], true));
    }

    #[test]
    fn easytab_lang_wins() {
        assert!(detect_with(
            &[("EASYTAB_LANG", "fr"), ("LANG", "en_US")],
            false
        ));
        assert!(!detect_with(
            &[("EASYTAB_LANG", "en"), ("LANG", "fr_FR")],
            true
        ));
    }

    #[test]
    fn tr_picks_one_of_the_two() {
        let x = 3;
        let text = tr!("{x} files", "{x} fichiers");
        assert!(text == "3 files" || text == "3 fichiers", "{text}");
        assert_eq!(text == "3 fichiers", fr());
        assert_eq!(tr("yes", "oui") == "oui", fr());
    }
}
