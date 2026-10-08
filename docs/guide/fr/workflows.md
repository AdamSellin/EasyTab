[English](../en/workflows.md) · **Français**

# Workflows

Des commandes enregistrées avec des champs à remplir, écrits entre accolades, dans
`~/.easytab/workflows.toml` (`easytab workflows` le crée avec des exemples) :

```toml
[[workflow]]
name = "Shell dans un conteneur"
command = "docker exec -it {conteneur} bash"

[[workflow]]
name = "Branche à partir de main"
command = "git switch -c {branche} main"
description = "Nouvelle branche, partie de main"   # facultatif
```

Un workflow est proposé dans la liste (icône ▸) dès qu'on tape le début de sa commande
(`docker ex`), et dans la recherche Ctrl+R par son nom ou sa commande. Le choisir écrit la
commande jusqu'au premier champ ; la suite s'affiche en gris (`{conteneur} bash`). On tape la
valeur du champ, avec l'aide de la liste habituelle (ici les conteneurs de docker), puis → ajoute
la suite jusqu'au champ suivant. `${HOME}` et `{a,b}` restent du texte pour le shell. Le fichier
est lu à l'ouverture du terminal ; `easytab doctor` y signale une erreur.

[← Sommaire](README.md)
