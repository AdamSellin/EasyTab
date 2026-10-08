# Specs de complétion

Générés à partir des specs Fig
([withfig/autocomplete](https://github.com/withfig/autocomplete), licence MIT,
voir `LICENSE-fig`) et embarqués, compressés (zlib), dans les binaires d'EasyTab :

- `specs.json.z` : la partie déclarative de toutes les specs de premier niveau (sous-commandes,
  options, valeurs, fichiers et dossiers). Chaque *generator* (suggestions calculées par du
  JavaScript : branches git, conteneurs docker…) y est remplacé par son chemin dans la spec
  d'origine. Au démarrage, EasyTab ne lit que le nom et la description de chaque commande ; le
  reste d'une spec est lu à la première complétion de la commande.
- `loadable.json.z` : les specs des sous-dossiers du paquet (`php/bin-console`,
  `dotnet/dotnet-build`…), par chemin. D'autres specs y renvoient par `loadSpec` ; elles ne
  sont pas proposées comme commandes, et ce fichier n'est décompressé qu'au premier `loadSpec`.
  Les sous-dossiers `aws/`, `az/` et `gcloud/` (des milliers de sous-commandes de services
  cloud) ne sont pas importés.
- `modules.json.z` : le code JavaScript (compilé, tel que publié par Fig) des specs qui ont
  des generators ou un `generateSpec` (sous-commandes calculées : celles de `composer`…).
  EasyTab l'exécute dans son moteur JS embarqué. Les specs produites par `generateSpec` sont
  converties par le même code que l'import (`crates/easytab-core/src/fig-convert.js`).

S'y ajoute `windows.json`, écrit à la main (JSON lisible, non compressé, au même format) : les
outils de Windows absents des specs Fig (winget, robocopy, taskkill, netsh…), d'après leur aide
(`robocopy /?`, `winget install --help`…). Il n'est utilisé que sous Windows, où ses specs passent
avant celles de Fig de même nom (`ping`, `where`). Une option nommée `/LOG:` ou `/scanfile=`
prend sa valeur collée.

`easytab.json`, also written by hand in the same format, describes EasyTab's own `easytab`
command, on every system. A test of `easytab-cli` (`easytab_spec_matches_the_command`) checks it
against the command's clap definition: a new subcommand or option goes in both.

Seules les commandes installées (présentes dans le PATH) sont proposées comme commandes.

## Régénérer

```sh
npm pack @withfig/autocomplete
tar xzf withfig-autocomplete-*.tgz
node tools/import-fig-specs.mjs package specs
```

Toutes les specs de `package/build/*.js` sont importées, sauf celles de `SKIP`, ainsi que celles
des sous-dossiers, sauf ceux de `SKIP_DIRS`.
