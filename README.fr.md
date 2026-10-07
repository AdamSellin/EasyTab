<p align="center">
  <img src="docs/images/logo.svg" alt="Logo d'EasyTab" width="96">
</p>

<h1 align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/images/title-dark.svg">
    <img alt="EasyTab" src="docs/images/title-light.svg" height="56">
  </picture>
</h1>

<p align="center"><a href="README.md">English</a> · <b>Français</b></p>

<p align="center">
  L'autocomplétion façon IDE pour ton terminal.<br>
  Windows, macOS et Linux · PowerShell, bash, zsh et Git Bash.
</p>

<p align="center">
  <img src="docs/images/easytab.png" alt="La liste d'EasyTab sous le curseur, dans Windows Terminal" width="720">
</p>

---

Pendant que tu tapes une commande, EasyTab ouvre une petite liste sous le curseur : sous-commandes,
options, branches git, scripts npm, fichiers… avec leur description. Tu choisis, Tab, c'est écrit.

- **Plus de 700 commandes** connues, grâce aux specs de [Fig](https://github.com/withfig/autocomplete) :
  git, docker, npm, kubectl, composer, symfony…
- **Les valeurs de ton projet** : branches, scripts de `package.json`, cibles du `Makefile`,
  hôtes SSH, conteneurs docker.
- **Ton historique** en tête de liste (d'abord les commandes lancées dans le dossier courant),
  les suggestions que tu choisis le plus souvent, et Ctrl+R pour y chercher.
- **Les fautes de frappe corrigées** : après l'échec de `gti status`, `git status` attend en gris
  au prompt suivant.
- **Des workflows** : commandes enregistrées avec des champs à remplir, comme
  `docker exec -it {conteneur} bash`.
- **Ton shell** : les alias (`g push` se complète comme `git push`), les variables
  d'environnement (`$HOME`) et, pour les commandes sans spec, les complétions de bash-completion
  ou de fish.
- **Rien à apprendre** : le terminal et le shell restent les tiens, la liste ne prend jamais le
  clavier.

<p align="center">
  <img src="docs/images/demo.gif" alt="git checkout, une branche, npm run dev et cd src/ complétés avec la liste" width="720">
</p>

## Installer

```sh
# macOS, Linux
curl -fsSL https://github.com/AdamSellin/EasyTab/releases/latest/download/install.sh | sh
```

```powershell
# Windows (PowerShell et Git Bash)
irm https://github.com/AdamSellin/EasyTab/releases/latest/download/install.ps1 | iex
```

<p align="center">
  <img src="docs/images/install.gif" alt="Installation d'EasyTab en une commande dans PowerShell, puis easytab doctor" width="720">
</p>

Ouvre ensuite un nouveau terminal. Pour mettre à jour : `easytab update`.
Sur un PC d'entreprise qui bloque le script : [installer sans script](docs/guide.md#pc-dentreprise).

## Utiliser

| Touche | Action |
|---|---|
| ↑ ↓, Maj+Tab | Choisir une suggestion |
| Tab | L'écrire |
| Entrée | L'écrire si elle complète le mot tapé, sinon lancer la commande |
| Échap | Fermer la liste |
| Ctrl+Espace | La rouvrir après Échap |
| → | Accepter la suggestion en gris (historique, correction, commande suivante comme `git push`, suite d'un workflow) |
| Ctrl+R | Chercher dans tout l'historique |

| Commande | |
|---|---|
| `easytab config` | Ouvre les réglages (`~/.easytab/config.toml`) |
| `easytab workflows` | Ouvre tes workflows (`~/.easytab/workflows.toml`) |
| `easytab doctor` | Vérifie l'installation |
| `easytab update` | Installe la dernière version |
| `easytab uninstall` | Retire EasyTab du shell |

Les messages sont en français si le système l'est, sinon en anglais (`EASYTAB_LANG=fr` pour forcer).

Tout le reste (réglages, specs personnelles, compilation) est dans le [guide](docs/guide.md).

## Signature du code

Signature de code gratuite fournie par [SignPath.io](https://about.signpath.io), certificat de
[SignPath Foundation](https://signpath.org) (*Free code signing provided by SignPath.io,
certificate by SignPath Foundation*). Les programmes Windows et les scripts PowerShell de chaque
version sont compilés et signés par le [workflow Release](.github/workflows/release.yml), sur les
machines de GitHub. La signature est en cours de mise en place : les versions actuelles ne sont
pas encore signées.

- Contributeurs et relecteurs : [Adam Sellin](https://github.com/AdamSellin)
- Approbateurs : [Adam Sellin](https://github.com/AdamSellin)

Confidentialité : ce programme n'envoie aucune information à d'autres systèmes en réseau, sauf à
la demande expresse de l'utilisateur ou de la personne qui l'installe ou l'utilise. Seul
`easytab update` contacte GitHub, pour télécharger la dernière version.

## Licence

MIT. Les specs de complétion viennent de [withfig/autocomplete](https://github.com/withfig/autocomplete)
(MIT, voir [specs/LICENSE-fig](specs/LICENSE-fig)).
