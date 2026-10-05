// Convertit les specs Fig (@withfig/autocomplete, licence MIT) en JSON statique
// lisible par easytab-core.
//
// Usage : node tools/import-fig-specs.mjs <dossier du paquet @withfig/autocomplete> specs
//
// Importe toutes les specs de premier niveau du paquet, et celles des
// sous-dossiers (`php/bin-console`, `dotnet/dotnet-add`…), que d'autres specs
// chargent par `loadSpec`. Écrit trois fichiers, compressés (zlib), dans le
// dossier de sortie :
// - `specs.json.z` : la partie déclarative des specs de premier niveau
//   (sous-commandes, options, valeurs, fichiers et dossiers). Chaque generator
//   dynamique y est remplacé par son chemin dans la spec d'origine.
// - `loadable.json.z` : les specs des sous-dossiers, par chemin, au même
//   format. Elles ne sont pas proposées comme commandes, et ne sont
//   décompressées qu'au premier `loadSpec`.
// - `modules.json.z` : le code JavaScript des specs qui ont des generators ou
//   un `generateSpec`. EasyTab l'exécute dans son moteur JS embarqué.
//
// La conversion elle-même est dans `crates/easytab-core/src/fig-convert.js`,
// que le moteur embarqué utilise aussi pour les specs produites par
// `generateSpec`.

import { readFileSync, readdirSync, writeFileSync } from "node:fs";
import { deflateSync } from "node:zlib";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { runInThisContext } from "node:vm";

const here = dirname(fileURLToPath(import.meta.url));
runInThisContext(
  readFileSync(join(here, "../crates/easytab-core/src/fig-convert.js"), "utf8"),
);
const convert = globalThis.__easytab_convert;

// Fichiers du paquet qui ne sont pas des specs de commande.
const SKIP = new Set(["-", "index"]);

// Sous-dossiers ignorés. `aws/` (plus de 400 specs, 36 Mo), `az/` (13 Mo) et
// `gcloud/` (14 Mo, 600 Ko une fois compressé) décrivent les milliers de
// sous-commandes de services cloud : ils feraient plus que doubler la taille des
// specs embarquées pour trois commandes, qui gardent la liste de leurs services
// (spec de premier niveau) sans le détail. `example/` : exemples de Fig.
const SKIP_DIRS = new Set(["aws", "az", "gcloud", "example"]);

// Specs dont les suggestions viennent d'une fonction JS qu'on remplace par un
// équivalent déclaratif.
const OVERRIDES = {
  // Le generator de `cd` liste les dossiers.
  cd: (spec) => {
    spec.args[0].templates = ["folders"];
    delete spec.args[0].generators;
  },
};

const hasCode = (spec) => {
  const json = JSON.stringify(spec);
  return json.includes('"generators":') || json.includes('"generate":');
};

const root = resolve(process.argv[2] ?? "");
const out = resolve(process.argv[3] ?? "specs");
const build = join(root, "build");
const { version } = JSON.parse(readFileSync(join(root, "package.json"), "utf8"));

// Chemins des specs (sans `.js`), relatifs à `build`.
function list(dir, prefix) {
  const found = [];
  for (const entry of readdirSync(join(build, dir), { withFileTypes: true })) {
    if (entry.isDirectory()) {
      if (SKIP_DIRS.has(entry.name)) continue;
      found.push(...list(join(dir, entry.name), `${prefix}${entry.name}/`));
    } else if (entry.name.endsWith(".js") && !SKIP.has(entry.name.slice(0, -3))) {
      found.push(prefix + entry.name.slice(0, -3));
    }
  }
  return found.sort();
}

const specs = [];
const loadable = {};
const modules = {};
for (const name of list("", "")) {
  const file = join(build, `${name}.js`);
  const module = await import(pathToFileURL(file).href);
  // Les specs versionnées (`heroku/index`) et les fichiers d'aide
  // (`deno/generators`) n'exportent pas d'objet.
  const spec = typeof module.default === "object" ? convert(module.default) : undefined;
  if (!spec) {
    console.error(`ignorée (pas une spec statique) : ${name}`);
    continue;
  }
  OVERRIDES[name]?.(spec);
  if (hasCode(spec)) {
    spec.module = name;
    modules[name] = readFileSync(file, "utf8");
  }
  if (name.includes("/")) loadable[name] = spec;
  else specs.push(spec);
}

// Les `loadSpec` vers une spec absente ne donnent rien : on les signale.
const known = new Set([...Object.keys(loadable), ...specs.flatMap((s) => s.names)]);
const missing = new Set();
const check = (node) => {
  if (!node || typeof node !== "object") return;
  if (typeof node.load === "string" && !known.has(node.load)) missing.add(node.load);
  for (const value of Object.values(node)) check(value);
};
check(specs);
check(loadable);
const skipped = [...missing].filter((m) => !SKIP_DIRS.has(m.split("/")[0]));
if (skipped.length > 0) console.error(`loadSpec introuvables : ${skipped.join(", ")}`);

const source = `@withfig/autocomplete@${version}`;
const write = (file, value) =>
  writeFileSync(join(out, file), deflateSync(JSON.stringify(value), { level: 9 }));
write("specs.json.z", { source, specs });
write("loadable.json.z", { source, specs: loadable });
write("modules.json.z", { source, modules });
console.error(
  `${specs.length} specs exportées (+ ${Object.keys(loadable).length} chargées par loadSpec), ` +
    `${Object.keys(modules).length} avec du code JS`,
);
