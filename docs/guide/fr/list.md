[English](../en/list.md) · **Français**

# La liste

| Touche | Action |
|---|---|
| ↑ / ↓, Maj+Tab | Choisir une suggestion (Maj+Tab remonte) |
| Tab | Insérer la suggestion choisie |
| Entrée | Insérer la suggestion surlignée quand elle complète le mot tapé, après ↑ / ↓, quand c'est une sous-commande et que rien n'est tapé dans le mot (`go ` Entrée → `build`), ou juste après avoir choisi une commande ou une sous-commande dans la liste (`dock` Entrée → `docker-compose `, Entrée → `up`) ; sinon la commande part comme d'habitude |
| Échap | Fermer la liste jusqu'à la prochaine frappe |
| Ctrl+Espace | Rouvrir la liste fermée avec Échap |
| → | Accepter la suggestion en gris |
| Ctrl+R | Chercher dans tout l'historique (Ctrl+R de nouveau : revenir à la liste habituelle) |

La suggestion en gris, après le curseur, est la commande la plus récente de l'historique qui
prolonge la ligne. Elle s'affiche dans le terminal, même avec la fenêtre flottante, et seulement
quand le curseur est en fin de ligne ; → l'accepte, sinon → déplace le curseur comme d'habitude.
Si le shell affiche déjà sa propre suggestion (prédictions de PowerShell), EasyTab n'ajoute pas
la sienne.

**Recherche (Ctrl+R).** La ligne tapée devient une recherche dans tout l'historique : les
commandes qui contiennent chacun de ses mots, dans n'importe quel ordre et sans tenir compte de la
casse, les plus récentes d'abord (`dock up` trouve `docker compose up -d`). La description donne le
dossier où la commande a été lancée, son échec éventuel, sa durée et sa date. Tab ou Entrée
remplace la ligne par la commande choisie, sans la lancer ; Échap ou Ctrl+R sort de la recherche.
Les workflows qui correspondent passent devant. `search = false` dans `[keys]` laisse Ctrl+R au
shell.

**Correction.** Quand une commande échoue sur une faute de frappe, la commande corrigée s'affiche
en gris au prompt suivant, et → l'accepte : `gti status` → `git status` (commande introuvable,
code 127 en bash et zsh, tout échec dans PowerShell), `git stauts` → `git status` (sous-commande
inconnue d'une spec). Une faute, c'est une lettre en trop, en moins, changée, ou deux lettres
inversées (deux fautes au plus pour un mot de plus de quatre lettres) ; à égalité, le nom le plus
utilisé gagne. Rien n'est proposé pour une ligne avec `|`, `;`, `&` ou `$`. `correct = false`
dans `[list]` le désactive.

**Commande suivante.** Après une commande réussie, la suite habituelle s'affiche en gris au
prompt suivant, et → l'accepte : `git push` après `git commit`, `git push origin v1.2` après
`git tag v1.2` (ou `git tag -a v1.2 …`), `git push -u origin nom` après `git switch -c nom` ou
`git checkout -b nom`. `next = false` dans `[list]` le désactive.

Quand la liste est fermée, Tab, Maj+Tab et Ctrl+Espace gardent leur comportement habituel
(complétion du shell), sauf Ctrl+Espace juste après Échap.

La liste s'affiche dans une fenêtre flottante (`easytab-overlay`), placée sous le
curseur du terminal : coins arrondis, ombre, icônes par type (branche git, script npm, dossier,
fichier, option…) et description en bas. La fenêtre ne prend jamais le focus : le clavier reste
au terminal. Si elle ne trouve pas le curseur (ou avec `EASYTAB_OVERLAY=0`), la liste est
dessinée dans le terminal. Pour comprendre un mauvais placement, `EASYTAB_OVERLAY_LOG=fichier`
journalise chaque position du curseur lue par la fenêtre.

- Windows : Windows Terminal et VS Code, curseur lu par UI Automation.
- macOS : curseur lu par l'accessibilité si le terminal y a accès (Réglages Système >
  Confidentialité et sécurité > Accessibilité : Terminal, iTerm…, dont `easytab-overlay` hérite) ;
  sinon, position déduite de la fenêtre du terminal et de sa taille en colonnes et lignes.
- Linux : sessions X11 seulement, position déduite de la fenêtre active. Sous Wayland, la liste
  reste dans le terminal (`EASYTAB_OVERLAY=1` essaie quand même, pour un terminal XWayland).

Dans le terminal, la liste montre une pastille colorée par type (`>`
commande, `$` sous-commande, `-` option, `@` valeur calculée, `/` dossier…), les lettres tapées en
gras, les arguments attendus en gris (`--cleanup <mode>`) et la description de la suggestion
choisie en bas. Avec `EASYTAB_ICONS=emoji`, les pastilles deviennent des emoji (📦 🚩 🌿 📁…).

[← Sommaire](README.md)
