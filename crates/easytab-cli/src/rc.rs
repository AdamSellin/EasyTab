//! Ajout et retrait du bloc EasyTab dans les fichiers de config des shells.

const BEGIN: &str = "# >>> easytab >>>";
const END: &str = "# <<< easytab <<<";

pub fn has_block(content: &str) -> bool {
    content.lines().any(|line| line.trim() == BEGIN)
}

/// Ajoute `line` dans deux blocs : en tête de fichier, pour relancer le shell
/// sous EasyTab avant de lire le reste de la config, et en fin de fichier, pour
/// poser les hooks après les thèmes de prompt.
pub fn add_blocks(content: &str, line: &str) -> String {
    let block = format!("{BEGIN}\n{line}\n{END}\n");
    let mut out = block.clone();
    if !content.is_empty() {
        out.push('\n');
        out.push_str(content);
        if !content.ends_with('\n') {
            out.push('\n');
        }
        out.push('\n');
        out.push_str(&block);
    }
    out
}

/// Retire tous les blocs EasyTab et les lignes vides ajoutées autour.
pub fn remove_blocks(content: &str) -> String {
    let mut out = Vec::new();
    let mut inside = false;
    let mut after_block = false;
    for line in content.lines() {
        match line.trim() {
            BEGIN => {
                inside = true;
                // Ligne vide ajoutée avant le bloc de fin.
                if out.last().is_some_and(|l: &&str| l.trim().is_empty()) {
                    out.pop();
                }
            }
            END if inside => {
                inside = false;
                after_block = true;
            }
            _ if inside => {}
            // Ligne vide ajoutée après le bloc de tête.
            "" if after_block && out.is_empty() => after_block = false,
            _ => {
                after_block = false;
                out.push(line);
            }
        }
    }
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

/// Entoure de guillemets simples pour PowerShell.
pub fn powershell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINE: &str = "eval \"$('/usr/bin/easytab' init zsh)\"";

    fn block() -> String {
        format!("{BEGIN}\n{LINE}\n{END}\n")
    }

    #[test]
    fn adds_blocks_at_top_and_bottom_then_removes_them() {
        let original = "export PATH=$HOME/bin:$PATH\nalias ll='ls -l'\n";
        let installed = add_blocks(original, LINE);
        assert!(has_block(&installed));
        assert_eq!(installed, format!("{}\n{original}\n{}", block(), block()));
        assert_eq!(remove_blocks(&installed), original);
    }

    #[test]
    fn handles_empty_file_and_missing_newline() {
        assert_eq!(add_blocks("", LINE), block());
        assert_eq!(
            add_blocks("alias g=git", LINE),
            format!("{}\nalias g=git\n\n{}", block(), block())
        );
        assert_eq!(remove_blocks(&add_blocks("", LINE)), "");
        assert_eq!(
            remove_blocks(&add_blocks("alias g=git", LINE)),
            "alias g=git\n"
        );
    }

    #[test]
    fn removes_the_older_single_block_at_the_end() {
        let content = format!("a\n\n{}", block());
        assert_eq!(remove_blocks(&content), "a\n");
    }

    #[test]
    fn keeps_lines_after_a_block() {
        let content = format!("a\n{BEGIN}\n{LINE}\n{END}\nb\n");
        assert_eq!(remove_blocks(&content), "a\nb\n");
    }

    #[test]
    fn quotes_paths() {
        assert_eq!(shell_quote("/opt/my app/easytab"), "'/opt/my app/easytab'");
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
    }
}
