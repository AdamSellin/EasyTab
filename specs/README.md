# Specs de complétion

`specs.json` est généré à partir des specs Fig
([withfig/autocomplete](https://github.com/withfig/autocomplete), licence MIT,
voir `LICENSE-fig`). Il est embarqué dans les binaires d'EasyTab.

- `specs.json` : la partie déclarative des specs (sous-commandes, options, valeurs,
  fichiers et dossiers). Chaque *generator* (suggestions calculées par du JavaScript :
  branches git, conteneurs docker…) y est remplacé par son chemin dans la spec d'origine.
- `modules.json` : le code JavaScript (compilé, tel que publié par Fig) des specs qui ont
  des generators. EasyTab l'exécute dans son moteur JS embarqué.

## Régénérer

```sh
npm pack @withfig/autocomplete
tar xzf withfig-autocomplete-*.tgz
node tools/import-fig-specs.mjs package specs
```

La liste des commandes embarquées est la constante `COMMANDS` de
`tools/import-fig-specs.mjs`.
