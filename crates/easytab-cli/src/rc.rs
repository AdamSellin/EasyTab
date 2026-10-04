//! Ajout et retrait du bloc EasyTab dans les fichiers de config des shells.

const BEGIN: &str = "# >>> easytab >>>";
const END: &str = "# <<< easytab <<<";

pub fn has_block(content: &str) -> bool {
    content.lines().any(|line| line.trim() == BEGIN)
}

/// Ajoute le bloc en fin de fichier, pour passer après les thèmes de prompt.
pub fn add_block(content: &str, line: &str) -> String {
    let mut out = content.to_string();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    if !out.is_empty() {
        out.push('\n');
    }
    out.push_str(&format!("{BEGIN}\n{line}\n{END}\n"));
    out
}

pub fn remove_block(content: &str) -> String {
    let mut out = Vec::new();
    let mut inside = false;
    for line in content.lines() {
        match line.trim() {
            BEGIN => inside = true,
            END if inside => inside = false,
            _ if !inside => out.push(line),
            _ => {}
        }
    }
    // Retire la ligne vide ajoutée avant le bloc.
    while out.last().is_some_and(|line| line.trim().is_empty()) {
        out.pop();
    }
    if out.is_empty() {
        String::new()
    } else {
        out.join("\n") + "\n"
    }
}

/// Entoure de guillemets simples pour un shell POSIX.
pub fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINE: &str = "eval \"$('/usr/bin/easytab' init zsh)\"";

    #[test]
    fn add_then_remove_restores_file() {
        let original = "export PATH=$HOME/bin:$PATH\nalias ll='ls -l'\n";
        let installed = add_block(original, LINE);
        assert!(has_block(&installed));
        assert!(installed.ends_with(&format!("{BEGIN}\n{LINE}\n{END}\n")));
        assert_eq!(remove_block(&installed), original);
    }

    #[test]
    fn handles_empty_file_and_missing_newline() {
        assert_eq!(add_block("", LINE), format!("{BEGIN}\n{LINE}\n{END}\n"));
        assert_eq!(
            add_block("alias g=git", LINE),
            format!("alias g=git\n\n{BEGIN}\n{LINE}\n{END}\n")
        );
        assert_eq!(remove_block(&add_block("", LINE)), "");
    }

    #[test]
    fn keeps_lines_after_the_block() {
        let content = format!("a\n{BEGIN}\n{LINE}\n{END}\nb\n");
        assert_eq!(remove_block(&content), "a\nb\n");
    }

    #[test]
    fn quotes_paths() {
        assert_eq!(shell_quote("/opt/my app/easytab"), "'/opt/my app/easytab'");
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
    }
}
