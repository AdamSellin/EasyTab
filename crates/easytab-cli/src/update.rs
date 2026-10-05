//! `easytab update` : télécharge la dernière version publiée sur GitHub et la
//! réinstalle. Comme les scripts d'installation, il passe par `curl` et `tar`
//! (fournis par Windows 10+, macOS et Linux), et par l'API GitHub avec un
//! jeton quand le dépôt est privé.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};
use serde::Deserialize;

const REPO: &str = "AdamSellin/EasyTab";

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    /// Adresse de l'API : marche aussi pour un dépôt privé, avec un jeton.
    url: String,
    browser_download_url: String,
}

/// Plateforme de ce programme, comme dans le nom des archives publiées.
fn target() -> Result<&'static str> {
    Ok(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => "x86_64-pc-windows-msvc",
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        (os, arch) => bail!("pas de version publiée pour {os} {arch}"),
    })
}

fn archive_name(target: &str) -> String {
    let extension = if target.contains("windows") {
        "zip"
    } else {
        "tar.gz"
    };
    format!("easytab-{target}.{extension}")
}

/// Vrai si `tag` (`v0.1.2`) n'est pas plus récent que `current` (`0.1.2`).
fn is_current(tag: &str, current: &str) -> bool {
    let numbers = |version: &str| -> Vec<u64> {
        version
            .trim_start_matches('v')
            .split(['.', '-'])
            .map_while(|part| part.parse().ok())
            .collect()
    };
    numbers(tag) <= numbers(current)
}

/// Sous Windows, ceux du système : le `tar` de Git Bash ne lit pas les zip.
fn system_tool(name: &str) -> PathBuf {
    if cfg!(windows) {
        if let Some(root) = std::env::var_os("SystemRoot") {
            let path = Path::new(&root)
                .join("System32")
                .join(format!("{name}.exe"));
            if path.is_file() {
                return path;
            }
        }
    }
    PathBuf::from(name)
}

/// Jeton GitHub : `$GITHUB_TOKEN`, sinon celui que git utilise déjà.
fn github_token() -> Option<String> {
    if let Ok(token) = std::env::var("GITHUB_TOKEN") {
        if !token.is_empty() {
            return Some(token);
        }
    }
    let mut child = Command::new("git")
        .args(["credential", "fill"])
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    {
        use std::io::Write;
        let mut stdin = child.stdin.take()?;
        let _ = stdin.write_all(b"protocol=https\nhost=github.com\n\n");
    }
    let output = child.wait_with_output().ok()?;
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .find_map(|line| line.strip_prefix("password="))
        .map(str::to_string)
        .filter(|token| !token.is_empty())
}

/// `curl` silencieux qui échoue sur une erreur HTTP. `curl` ne renvoie pas
/// l'en-tête `Authorization` quand GitHub redirige vers un autre domaine.
fn curl(token: Option<&str>, accept: &str, url: &str, output: Option<&Path>) -> Result<Vec<u8>> {
    let mut command = Command::new(system_tool("curl"));
    command.args(["-fsSL", "-H", &format!("Accept: {accept}")]);
    if let Some(token) = token {
        command.args(["-H", &format!("Authorization: Bearer {token}")]);
    }
    if let Some(output) = output {
        command.arg("-o").arg(output);
    }
    let result = command
        .arg(url)
        .stderr(Stdio::null())
        .output()
        .context("curl introuvable")?;
    if !result.status.success() {
        bail!("échec du téléchargement de {url}");
    }
    Ok(result.stdout)
}

/// Dernière version publiée, et le jeton qui a permis de la lire (dépôt privé).
fn latest_release() -> Result<(Release, Option<String>)> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let accept = "application/vnd.github+json";
    let (body, token) = match curl(None, accept, &url, None) {
        Ok(body) => (body, None),
        Err(_) => {
            let token = github_token().context(
                "dépôt privé : connecte git à GitHub, ou définis GITHUB_TOKEN, puis relance",
            )?;
            let body = curl(Some(&token), accept, &url, None)
                .context("impossible de lire la dernière version sur GitHub")?;
            (body, Some(token))
        }
    };
    let release = serde_json::from_slice(&body).context("réponse de GitHub illisible")?;
    Ok((release, token))
}

pub fn run(force: bool, installed_shells: &[&str]) -> Result<()> {
    let current = env!("CARGO_PKG_VERSION");
    let target = target()?;
    let (release, token) = latest_release()?;
    if is_current(&release.tag_name, current) && !force {
        println!("EasyTab est à jour ({}).", release.tag_name);
        return Ok(());
    }
    let name = archive_name(target);
    let asset = release
        .assets
        .iter()
        .find(|asset| asset.name == name)
        .with_context(|| format!("{name} absent de la version {}", release.tag_name))?;

    let dir = std::env::temp_dir().join(format!("easytab-update-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).with_context(|| format!("création de {}", dir.display()))?;
    let result = download_and_install(&dir, asset, token.as_deref(), target, installed_shells);
    let _ = std::fs::remove_dir_all(&dir);
    result?;
    println!(
        "EasyTab mis à jour : {current} → {}. Rouvre tes terminaux pour l'utiliser.",
        release.tag_name
    );
    Ok(())
}

fn download_and_install(
    dir: &Path,
    asset: &Asset,
    token: Option<&str>,
    target: &str,
    installed_shells: &[&str],
) -> Result<()> {
    let archive = dir.join(&asset.name);
    println!("Téléchargement de {}", asset.name);
    // Lien direct d'abord ; dépôt privé : par l'API, avec un jeton.
    let direct = match token {
        Some(_) => None,
        None => curl(None, "*/*", &asset.browser_download_url, Some(&archive)).ok(),
    };
    if direct.is_none() {
        let token = token.map(str::to_string).or_else(github_token);
        curl(
            token.as_deref(),
            "application/octet-stream",
            &asset.url,
            Some(&archive),
        )
        .context("dépôt privé : connecte git à GitHub, ou définis GITHUB_TOKEN, puis relance")?;
    }
    let status = Command::new(system_tool("tar"))
        .arg("-xf")
        .arg(&archive)
        .arg("-C")
        .arg(dir)
        .status()
        .context("tar introuvable")?;
    if !status.success() {
        bail!("décompression de {} impossible", asset.name);
    }
    let easytab = dir
        .join(format!("easytab-{target}"))
        .join(format!("easytab{}", std::env::consts::EXE_SUFFIX));
    // La nouvelle version s'installe elle-même, pour chaque shell déjà
    // configuré : ses blocs remplacent les anciens.
    let shells: Vec<Option<&str>> = if installed_shells.is_empty() {
        vec![None]
    } else {
        installed_shells.iter().copied().map(Some).collect()
    };
    for shell in shells {
        let mut command = Command::new(&easytab);
        command.arg("install");
        if let Some(shell) = shell {
            command.args(["--shell", shell]);
        }
        let status = command
            .status()
            .with_context(|| format!("lancement de {}", easytab.display()))?;
        if !status.success() {
            bail!("l'installation de la nouvelle version a échoué");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_archives_like_the_release() {
        assert_eq!(
            archive_name("x86_64-pc-windows-msvc"),
            "easytab-x86_64-pc-windows-msvc.zip"
        );
        assert_eq!(
            archive_name("aarch64-apple-darwin"),
            "easytab-aarch64-apple-darwin.tar.gz"
        );
        assert!(target().is_ok());
    }

    #[test]
    fn compares_versions() {
        assert!(is_current("v0.1.2", "0.1.2"));
        assert!(is_current("0.1.2", "0.1.2"));
        assert!(!is_current("v0.1.3", "0.1.2"));
        assert!(!is_current("v0.10.0", "0.9.1"));
        // Jamais de retour en arrière sans --force.
        assert!(is_current("v0.1.1", "0.1.2"));
    }

    #[test]
    fn reads_github_releases() {
        let release: Release = serde_json::from_str(
            r#"{"tag_name": "v0.1.1", "name": "EasyTab v0.1.1", "assets": [
                {"name": "install.sh", "url": "https://api.github.com/repos/a/b/releases/assets/1",
                 "browser_download_url": "https://github.com/a/b/releases/download/v0.1.1/install.sh",
                 "size": 10}]}"#,
        )
        .unwrap();
        assert_eq!(release.tag_name, "v0.1.1");
        assert_eq!(release.assets[0].name, "install.sh");
    }
}
