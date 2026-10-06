//! Réglages de l'utilisateur, lus dans `~/.easytab/config.toml`. Tout est
//! facultatif : une clé absente garde sa valeur par défaut. Les variables
//! d'environnement (`EASYTAB_OVERLAY`, `EASYTAB_ICONS`) restent prioritaires.

use std::path::{Path, PathBuf};

use serde::Deserialize;

/// Fichier créé par `easytab config`, avec chaque réglage commenté, dans la
/// langue de l'utilisateur.
pub fn template() -> &'static str {
    crate::lang::tr(TEMPLATE, TEMPLATE_FR)
}

/// Modèle en anglais.
pub const TEMPLATE: &str = r##"# EasyTab settings. Remove the "#" in front of a line to change it.
# Terminals that are already open keep the old settings: reopen them.

[list]
# Number of suggestions visible before scrolling (3 to 20).
# rows = 8

# Fig-style floating window (true) or list drawn inside the terminal (false).
# overlay = true

# Theme: "dark" or "light".
# theme = "dark"

# Icons of the list drawn in the terminal: "badges" or "emoji".
# icons = "badges"

# Suggest commands typed before (shell history).
# history = true

# Commands without a spec: read their options from "command --help"
# (run once in the background, answer kept in ~/.easytab/cache).
# help = true

[keys]
# Enter inserts the highlighted suggestion when it completes the typed word
# or was picked with Up/Down (true), or always runs the command, only Tab
# inserting (false).
# enter_inserts = true
"##;

/// Modèle en français.
pub const TEMPLATE_FR: &str = r#"# Réglages d'EasyTab. Retirez le « # » devant une ligne pour la changer.
# Les terminaux déjà ouverts gardent les anciens réglages : rouvrez-les.

[list]
# Nombre de suggestions visibles avant de faire défiler (de 3 à 20).
# rows = 8

# Fenêtre flottante façon Fig (true) ou liste dessinée dans le terminal (false).
# overlay = true

# Thème : "dark" (sombre) ou "light" (clair).
# theme = "dark"

# Icônes de la liste dans le terminal : "badges" ou "emoji".
# icons = "badges"

# Proposer les commandes déjà tapées (historique du shell).
# history = true

# Commandes sans spec : lire leurs options dans « commande --help »
# (lancé une fois en arrière-plan, réponse gardée dans ~/.easytab/cache).
# help = true

[keys]
# Entrée insère la suggestion surlignée quand elle complète le mot tapé ou
# qu'on l'a choisie avec ↑/↓ (true), ou lance toujours la commande,
# seul Tab insérant (false).
# enter_inserts = true
"#;

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub list: List,
    pub keys: Keys,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct List {
    pub rows: usize,
    pub overlay: bool,
    pub theme: Theme,
    pub icons: Icons,
    pub history: bool,
    pub help: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Keys {
    pub enter_inserts: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    #[default]
    Dark,
    Light,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Icons {
    #[default]
    Badges,
    Emoji,
}

pub const MIN_ROWS: usize = 3;
pub const MAX_ROWS: usize = 20;

impl Default for List {
    fn default() -> Self {
        Self {
            rows: 8,
            overlay: true,
            theme: Theme::Dark,
            icons: Icons::Badges,
            history: true,
            help: true,
        }
    }
}

impl Default for Keys {
    fn default() -> Self {
        Self {
            enter_inserts: true,
        }
    }
}

impl Config {
    /// `~/.easytab/config.toml`.
    pub fn path() -> Option<PathBuf> {
        let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
        Some(PathBuf::from(home).join(".easytab").join("config.toml"))
    }

    /// Réglages du fichier, ou ceux par défaut s'il est absent ou invalide.
    pub fn load() -> Self {
        let file = Self::path().map(|path| Self::read(&path));
        let mut config = match file {
            Some(Ok(Some(config))) => config,
            _ => Self::default(),
        };
        config.apply_env(|name| std::env::var(name).ok());
        config
    }

    /// Lit le fichier : `Ok(None)` s'il n'existe pas, `Err` avec un message
    /// lisible s'il est invalide.
    pub fn read(path: &Path) -> Result<Option<Self>, String> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text).map(Some),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        let mut config: Self = toml::from_str(text).map_err(|e| e.to_string())?;
        config.list.rows = config.list.rows.clamp(MIN_ROWS, MAX_ROWS);
        Ok(config)
    }

    /// Les variables d'environnement passent avant le fichier.
    fn apply_env(&mut self, var: impl Fn(&str) -> Option<String>) {
        match var("EASYTAB_OVERLAY").as_deref() {
            Some("0") => self.list.overlay = false,
            Some("1") => self.list.overlay = true,
            _ => {}
        }
        match var("EASYTAB_ICONS").as_deref() {
            Some("emoji") => self.list.icons = Icons::Emoji,
            Some("badges") => self.list.icons = Icons::Badges,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_file_gives_defaults() {
        assert_eq!(Config::parse("").unwrap(), Config::default());
    }

    #[test]
    fn template_is_valid_and_matches_defaults() {
        for template in [TEMPLATE, TEMPLATE_FR] {
            assert_eq!(Config::parse(template).unwrap(), Config::default());
            // Tous les réglages décommentés : toujours valide.
            let uncommented = template
                .replace("# rows", "rows")
                .replace("# overlay", "overlay");
            let uncommented = uncommented
                .replace("# theme", "theme")
                .replace("# icons", "icons")
                .replace("# history", "history")
                .replace("# help", "help")
                .replace("# enter_inserts", "enter_inserts");
            assert_eq!(Config::parse(&uncommented).unwrap(), Config::default());
        }
        assert!(template() == TEMPLATE || template() == TEMPLATE_FR);
    }

    #[test]
    fn reads_settings() {
        let config = Config::parse(
            "[list]\nrows = 12\noverlay = false\ntheme = \"light\"\nicons = \"emoji\"\nhelp = false\n\
             [keys]\nenter_inserts = false\n",
        )
        .unwrap();
        assert_eq!(config.list.rows, 12);
        assert!(!config.list.overlay);
        assert_eq!(config.list.theme, Theme::Light);
        assert_eq!(config.list.icons, Icons::Emoji);
        assert!(!config.list.help);
        assert!(!config.keys.enter_inserts);
    }

    #[test]
    fn keeps_rows_in_range() {
        assert_eq!(
            Config::parse("[list]\nrows = 1").unwrap().list.rows,
            MIN_ROWS
        );
        assert_eq!(
            Config::parse("[list]\nrows = 99").unwrap().list.rows,
            MAX_ROWS
        );
    }

    #[test]
    fn reports_mistakes() {
        let error = Config::parse("[list]\nrow = 5").unwrap_err();
        assert!(error.contains("row"), "{error}");
        assert!(Config::parse("[list]\ntheme = \"blue\"").is_err());
    }

    #[test]
    fn environment_wins() {
        let mut config = Config::parse("[list]\noverlay = true\nicons = \"emoji\"").unwrap();
        config.apply_env(|name| match name {
            "EASYTAB_OVERLAY" => Some("0".into()),
            "EASYTAB_ICONS" => Some("badges".into()),
            _ => None,
        });
        assert!(!config.list.overlay);
        assert_eq!(config.list.icons, Icons::Badges);
    }

    #[test]
    fn reads_a_missing_file_as_none() {
        assert_eq!(
            Config::read(Path::new("/nonexistent/easytab.toml")),
            Ok(None)
        );
    }
}
