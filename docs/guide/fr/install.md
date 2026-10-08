[English](../en/install.md) · **Français**

# Installer

Le script d'installation copie `easytab`, `easytab-term` et `easytab-overlay` dans
`~/.easytab/bin` et ajoute EasyTab à la config du shell, qui met aussi ce dossier dans le PATH.
Relancer la même commande met à jour ; `easytab update` aussi, pour les shells déjà configurés.
Chaque version publie un `SHA256SUMS` : les scripts d'installation et `easytab update`
vérifient l'archive avant de la décompresser (une archive différente arrête l'installation).
Les anciennes versions mises de côté par une mise à jour (`*.old-…` dans `~/.easytab/bin`) sont
effacées au lancement suivant d'un terminal.

Shells pris en charge : zsh, bash et PowerShell (7 et Windows PowerShell 5), et Git Bash sous
Windows, dans Windows Terminal ou le terminal de VS Code. Dans la fenêtre « Git Bash » par défaut
(mintty), qui ne fournit pas de console aux programmes Windows, EasyTab passe par `winpty`, livré
avec Git for Windows ; sans lui, le shell s'y lance sans EasyTab, avec un message.

PowerShell : le profil charge `~/.easytab/bin/easytab-profile.ps1` (`. 'chemin'`), copié par
`easytab install --shell pwsh` depuis l'archive ; `easytab init pwsh` l'affiche encore pour les
profils d'avant ce fichier. Il ne contient aucun chemin, pour que la version publiée puisse être
signée.

## PC d'entreprise

Les antivirus et AMSI se méfient de `irm … | iex` (télécharger puis exécuter), et une politique
d'exécution `AllSigned` ou `Restricted` refuse les scripts. Sans script :

1. télécharger `easytab-x86_64-pc-windows-msvc.zip` depuis la
   [dernière version](https://github.com/AdamSellin/EasyTab/releases/latest) et l'extraire ;
2. dans le dossier extrait : `.\easytab.exe install --shell pwsh` (et `--shell bash` pour Git
   Bash), puis ouvrir un nouveau terminal.

Avec `AllSigned`, les scripts doivent être signés, ce que les versions ne sont pas encore (voir
[Signature](development.md#signature-windows)). Si l'antivirus bloque quand même un programme, le signaler comme faux positif
(Microsoft : <https://www.microsoft.com/wdsi/filesubmission>) ou demander au service informatique
d'autoriser l'éditeur.

[← Sommaire](README.md)
