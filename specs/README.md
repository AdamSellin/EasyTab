# Specs de complétion

`specs.json` est généré à partir des specs Fig
([withfig/autocomplete](https://github.com/withfig/autocomplete), licence MIT,
voir `LICENSE-fig`). Il est embarqué dans les binaires d'EasyTab.

Seule la partie déclarative est gardée (sous-commandes, options, valeurs,
fichiers et dossiers). Les suggestions calculées par du JavaScript (branches git,
conteneurs docker…) viendront avec un runtime JS embarqué.

## Régénérer

```sh
npm pack @withfig/autocomplete
tar xzf withfig-autocomplete-*.tgz
node tools/import-fig-specs.mjs package > specs/specs.json
```

La liste des commandes embarquées est la constante `COMMANDS` de
`tools/import-fig-specs.mjs`.
