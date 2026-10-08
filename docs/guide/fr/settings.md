[English](../en/settings.md) · **Français**

# Réglages

`easytab config` crée `~/.easytab/config.toml`, avec chaque réglage commenté ; `easytab doctor`
signale une erreur dans ce fichier. Rouvrir les terminaux après un changement.

```toml
[list]
rows = 8            # suggestions visibles (de 3 à 20)
overlay = true      # fenêtre flottante, ou false pour la liste dans le terminal
theme = "dark"      # ou "light"
icons = "badges"    # ou "emoji" (liste dans le terminal)
history = true      # proposer les commandes déjà tapées
inline = true       # suggestion en gris après le curseur, acceptée avec →
shell = true        # commandes sans spec : complétions de bash-completion ou fish
help = true         # commandes sans spec : options lues dans « commande --help »
correct = true      # après une faute de frappe, commande corrigée en gris au prompt suivant
next = true         # après git commit, git push en gris au prompt suivant (et git tag…)

[keys]
enter_inserts = true  # false : Entrée lance toujours la commande, seul Tab insère
search = true         # Ctrl+R cherche dans l'historique ; false : Ctrl+R reste au shell
```

Les variables `EASYTAB_OVERLAY` et `EASYTAB_ICONS` passent avant le fichier.

Langue des messages (commande `easytab`, scripts d'installation, modèle de `config.toml`) :
anglais par défaut, français si le système l'est (`LC_ALL`, `LC_MESSAGES` ou `LANG` commençant
par `fr` ; sans ces variables, la langue de Windows ou de macOS). `EASYTAB_LANG=fr` ou
`EASYTAB_LANG=en` force le choix.

Désactiver : `EASYTAB_DISABLE=1` pour une session, `easytab uninstall` pour de bon.

[← Sommaire](README.md)
