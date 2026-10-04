//! Découpage de la ligne de commande comme le ferait un shell POSIX (simplifié).

/// Mot de la ligne : `value` sans guillemets ni échappements, `raw` tel que tapé.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub value: String,
    pub raw: String,
}

/// Mots de la dernière commande de la ligne (après `|`, `;`, `&&`…).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    /// Mots terminés (suivis d'un espace).
    pub words: Vec<Token>,
    /// Mot en cours de frappe, vide après un espace.
    pub current: Token,
}

pub fn parse(input: &str) -> Line {
    let mut words = Vec::new();
    let mut value = String::new();
    let mut raw = String::new();
    let mut in_word = false;
    let mut quote: Option<char> = None;
    let mut escaped = false;

    for c in input.chars() {
        if escaped {
            value.push(c);
            raw.push(c);
            escaped = false;
            continue;
        }
        match (quote, c) {
            (Some('\''), '\'') | (Some('"'), '"') => {
                quote = None;
                raw.push(c);
            }
            (Some('"'), '\\') | (None, '\\') => {
                escaped = true;
                in_word = true;
                raw.push(c);
            }
            (Some(_), _) => {
                value.push(c);
                raw.push(c);
            }
            (None, '\'' | '"') => {
                quote = Some(c);
                in_word = true;
                raw.push(c);
            }
            (None, ' ' | '\t') => {
                if in_word {
                    words.push(Token {
                        value: std::mem::take(&mut value),
                        raw: std::mem::take(&mut raw),
                    });
                    in_word = false;
                }
            }
            (None, '|' | ';' | '&' | '(' | ')') => {
                // Nouvelle commande.
                words.clear();
                value.clear();
                raw.clear();
                in_word = false;
            }
            (None, _) => {
                in_word = true;
                value.push(c);
                raw.push(c);
            }
        }
    }

    Line {
        words,
        current: Token { value, raw },
    }
}

/// Échappe une valeur pour l'insérer telle quelle dans la ligne de commande.
pub fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        if matches!(
            c,
            ' ' | '\t'
                | '\''
                | '"'
                | '\\'
                | '$'
                | '`'
                | '!'
                | '&'
                | '|'
                | ';'
                | '('
                | ')'
                | '<'
                | '>'
                | '*'
                | '?'
                | '['
                | ']'
                | '#'
        ) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values(line: &Line) -> Vec<&str> {
        line.words.iter().map(|t| t.value.as_str()).collect()
    }

    #[test]
    fn splits_words_and_current() {
        let line = parse("git commit -m");
        assert_eq!(values(&line), ["git", "commit"]);
        assert_eq!(line.current.value, "-m");

        let line = parse("git commit ");
        assert_eq!(values(&line), ["git", "commit"]);
        assert_eq!(line.current.value, "");
    }

    #[test]
    fn handles_quotes_and_escapes() {
        let line = parse(r#"cd "my dir" it\'s 'a b"#);
        assert_eq!(values(&line), ["cd", "my dir", "it's"]);
        assert_eq!(line.current.value, "a b");
        assert_eq!(line.current.raw, "'a b");
    }

    #[test]
    fn keeps_only_last_command() {
        let line = parse("cat log | grep -i err && git st");
        assert_eq!(values(&line), ["git"]);
        assert_eq!(line.current.value, "st");
    }

    #[test]
    fn escapes_special_characters() {
        assert_eq!(escape("my file (1).txt"), r"my\ file\ \(1\).txt");
        assert_eq!(escape("plain-name"), "plain-name");
    }
}
