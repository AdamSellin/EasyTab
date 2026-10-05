# Specs de complétion

Générés à partir des specs Fig
([withfig/autocomplete](https://github.com/withfig/autocomplete), licence MIT,
voir `LICENSE-fig`) et embarqués, compressés (zlib), dans les binaires d'EasyTab :

- `specs.json.z` : la partie déclarative de toutes les specs de premier niveau (sous-commandes,
  options, valeurs, fichiers et dossiers). Chaque *generator* (suggestions calculées par du
  JavaScript : branches git, conteneurs docker…) y est remplacé par son chemin dans la spec
  d'origine. Au démarrage, EasyTab ne lit que le nom et la description de chaque commande ; le
  reste d'une spec est lu à la première complétion de la commande.
- `modules.json.z` : le code JavaScript (compilé, tel que publié par Fig) des specs qui ont
  des generators. EasyTab l'exécute dans son moteur JS embarqué.

Seules les commandes installées (présentes dans le PATH) sont proposées comme commandes.

## Régénérer

```sh
npm pack @withfig/autocomplete
tar xzf withfig-autocomplete-*.tgz
node tools/import-fig-specs.mjs package specs
```

Toutes les specs de `package/build/*.js` sont importées, sauf celles de `SKIP`.
