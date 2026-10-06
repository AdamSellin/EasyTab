//! Workflows : commandes enregistrées avec des champs à remplir
//! (`docker exec -it {conteneur} bash`), lues dans `~/.easytab/workflows.toml`.
//!
//! Choisir un workflow dans la liste insère son texte jusqu'au premier champ.
//! Le reste s'affiche en gris après le curseur : on tape la valeur du champ
//! (la liste propose les conteneurs, branches… comme d'habitude), puis →
//! ajoute le texte qui suit, jusqu'au champ suivant.

use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Workflow {
    pub command: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    #[serde(default)]
    workflow: Vec<Workflow>,
}

/// Fichier créé par `easytab workflows`, dans la langue de l'utilisateur.
pub fn template() -> &'static str {
    crate::lang::tr(TEMPLATE, TEMPLATE_FR)
}

pub const TEMPLATE: &str = r##"# EasyTab workflows: saved commands with fields to fill in, written {like_this}.
# Type the start of the command, pick it in the list, type the value of the
# field, then press Right arrow to add the rest of the command.
# Remove the "#" in front of the lines of an example to use it.

# [[workflow]]
# name = "Shell in a container"
# command = "docker exec -it {container} bash"

# [[workflow]]
# name = "Create a branch from main"
# command = "git switch -c {branch} main"
# description = "New branch, starting from main"
"##;

pub const TEMPLATE_FR: &str = r##"# Workflows d'EasyTab : commandes enregistrées avec des champs à remplir,
# écrits {comme_ceci}. Tapez le début de la commande, choisissez-la dans la
# liste, tapez la valeur du champ, puis la flèche droite ajoute la suite.
# Retirez le « # » devant les lignes d'un exemple pour l'utiliser.

# [[workflow]]
# name = "Shell dans un conteneur"
# command = "docker exec -it {conteneur} bash"

# [[workflow]]
# name = "Branche à partir de main"
# command = "git switch -c {branche} main"
# description = "Nouvelle branche, partie de main"
"##;

/// `~/.easytab/workflows.toml`.
pub fn path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    Some(PathBuf::from(home).join(".easytab").join("workflows.toml"))
}

/// Workflows du fichier, aucun s'il est absent ou invalide.
pub fn load() -> Vec<Workflow> {
    path().and_then(|path| read(&path).ok()).unwrap_or_default()
}

/// Lit le fichier : vide s'il n'existe pas, `Err` avec un message lisible s'il
/// est invalide.
pub fn read(path: &Path) -> Result<Vec<Workflow>, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => parse(&text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e.to_string()),
    }
}

pub fn parse(text: &str) -> Result<Vec<Workflow>, String> {
    let file: File = toml::from_str(text).map_err(|e| e.to_string())?;
    Ok(file
        .workflow
        .into_iter()
        .filter(|w| !w.command.trim().is_empty() && !w.command.contains('\n'))
        .collect())
}

/// Morceau d'une commande : texte, ou champ à remplir.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Part {
    Text(String),
    Field(String),
}

/// Découpe `docker exec -it {conteneur} bash` en texte et champs. `${HOME}`
/// et `{a,b}` (syntaxe du shell) restent du texte.
pub fn parts(command: &str) -> Vec<Part> {
    let mut parts = Vec::new();
    let mut text = String::new();
    let mut rest = command;
    while let Some(open) = rest.find('{') {
        let after = &rest[open + 1..];
        let field = after.find('}').map(|close| &after[..close]).filter(|name| {
            !name.is_empty()
                && !rest[..open].ends_with('$')
                && !name.contains(|c: char| c.is_whitespace() || matches!(c, ',' | '{'))
        });
        match field {
            Some(name) => {
                text.push_str(&rest[..open]);
                if !text.is_empty() {
                    parts.push(Part::Text(std::mem::take(&mut text)));
                }
                parts.push(Part::Field(name.to_string()));
                rest = &after[name.len() + 1..];
            }
            None => {
                text.push_str(&rest[..=open]);
                rest = after;
            }
        }
    }
    text.push_str(rest);
    if !text.is_empty() {
        parts.push(Part::Text(text));
    }
    parts
}

impl Workflow {
    /// Texte avant le premier champ, inséré quand on choisit le workflow.
    pub fn head(&self) -> String {
        match parts(&self.command).into_iter().next() {
            Some(Part::Text(text)) => text,
            _ => String::new(),
        }
    }
}

fn render(parts: &[Part]) -> String {
    parts
        .iter()
        .map(|part| match part {
            Part::Text(text) => text.clone(),
            Part::Field(name) => format!("{{{name}}}"),
        })
        .collect()
}

/// Suggestion en gris pendant le remplissage : le texte montré, et ce que →
/// insère (rien tant que le champ est vide).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hint {
    pub shown: String,
    pub accept: Option<String>,
}

/// Workflow en cours de remplissage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fill {
    /// Ligne telle qu'elle était au début du champ en cours.
    prefix: String,
    /// Reste de la commande, qui commence par le champ en cours.
    rest: Vec<Part>,
}

impl Fill {
    /// `line` : la ligne après l'insertion du début de `command`. `None` si
    /// la commande n'a pas de champ.
    pub fn start(line: &str, command: &str) -> Option<Fill> {
        let mut rest = parts(command);
        if let Some(Part::Text(_)) = rest.first() {
            rest.remove(0);
        }
        (!rest.is_empty()).then(|| Fill {
            prefix: line.to_string(),
            rest,
        })
    }

    /// Suggestion en gris pour la ligne `input`, `None` quand le remplissage
    /// est fini (dernier champ commencé, ou ligne modifiée avant le champ).
    pub fn hint(&self, input: &str) -> Option<Hint> {
        let value = input.strip_prefix(&self.prefix)?;
        if value.trim().is_empty() {
            return Some(Hint {
                shown: render(&self.rest[..]).trim_start().to_string(),
                accept: None,
            });
        }
        let Some(Part::Text(text)) = self.rest.get(1) else {
            return None;
        };
        // La liste ajoute souvent un espace après la valeur choisie.
        let text = if value.ends_with(char::is_whitespace) {
            text.trim_start()
        } else {
            text
        };
        if text.is_empty() {
            return None;
        }
        Some(Hint {
            shown: format!("{text}{}", render(&self.rest[2..])),
            accept: Some(text.to_string()),
        })
    }

    /// → a inséré `accepted` à la fin de la ligne `input` : passe au champ
    /// suivant. `false` s'il n'y en a plus.
    pub fn advance(&mut self, input: &str, accepted: &str) -> bool {
        self.prefix = format!("{input}{accepted}");
        self.rest.drain(..self.rest.len().min(2));
        !self.rest.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> Part {
        Part::Text(s.to_string())
    }

    fn field(s: &str) -> Part {
        Part::Field(s.to_string())
    }

    #[test]
    fn splits_fields() {
        assert_eq!(
            parts("docker exec -it {conteneur} bash"),
            [text("docker exec -it "), field("conteneur"), text(" bash")]
        );
        assert_eq!(
            parts("scp {fichier} {hôte}:"),
            [
                text("scp "),
                field("fichier"),
                text(" "),
                field("hôte"),
                text(":")
            ]
        );
        // Syntaxe du shell : pas des champs.
        assert_eq!(
            parts("echo ${HOME} {a,b} {}"),
            [text("echo ${HOME} {a,b} {}")]
        );
    }

    #[test]
    fn reads_the_file() {
        let workflows = parse(
            "[[workflow]]\nname = \"Shell\"\ncommand = \"docker exec -it {conteneur} bash\"\n",
        )
        .unwrap();
        assert_eq!(workflows[0].head(), "docker exec -it ");
        assert_eq!(workflows[0].name.as_deref(), Some("Shell"));
        assert!(parse("[[workflow]]\ncmd = \"ls\"").is_err());
        for template in [TEMPLATE, TEMPLATE_FR] {
            assert_eq!(parse(template).unwrap(), []);
            let uncommented = template.replace("# [[", "[[").replace("# name", "name");
            let uncommented = uncommented
                .replace("# command", "command")
                .replace("# description", "description");
            assert_eq!(parse(&uncommented).unwrap().len(), 2);
        }
    }

    #[test]
    fn fills_fields_one_after_the_other() {
        let command = "scp {fichier} {hôte}:/tmp";
        let mut fill = Fill::start("scp ", command).unwrap();
        // Champ vide : le reste, sans rien à insérer.
        assert_eq!(
            fill.hint("scp "),
            Some(Hint {
                shown: "{fichier} {hôte}:/tmp".into(),
                accept: None
            })
        );
        // Valeur tapée : → ajoute l'espace, jusqu'au champ suivant.
        let hint = fill.hint("scp a.txt").unwrap();
        assert_eq!(hint.shown, " {hôte}:/tmp");
        assert_eq!(hint.accept.as_deref(), Some(" "));
        assert!(fill.advance("scp a.txt", " "));
        let hint = fill.hint("scp a.txt serveur").unwrap();
        assert_eq!(hint.accept.as_deref(), Some(":/tmp"));
        assert!(!fill.advance("scp a.txt serveur", ":/tmp"));
        // Ligne effacée avant le champ : le remplissage s'arrête.
        let fill = Fill::start("scp ", command).unwrap();
        assert_eq!(fill.hint("sc"), None);
    }

    #[test]
    fn last_field_ends_the_fill() {
        let fill = Fill::start("ssh ", "ssh {hôte}").unwrap();
        assert!(fill.hint("ssh ").is_some());
        assert_eq!(fill.hint("ssh web"), None);
        assert_eq!(Fill::start("ls -la", "ls -la"), None);
    }

    #[test]
    fn value_followed_by_a_space() {
        let fill = Fill::start("docker exec -it ", "docker exec -it {conteneur} bash").unwrap();
        let hint = fill.hint("docker exec -it web ").unwrap();
        assert_eq!(hint.accept.as_deref(), Some("bash"));
    }
}
