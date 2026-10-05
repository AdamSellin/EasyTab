// Convertit les specs Fig (@withfig/autocomplete, licence MIT) en JSON statique
// lisible par easytab-core.
//
// Usage : node tools/import-fig-specs.mjs <dossier du paquet @withfig/autocomplete> specs
//
// Importe toutes les specs de premier niveau du paquet. Écrit deux fichiers,
// compressés (zlib), dans le dossier de sortie :
// - `specs.json.z` : la partie déclarative des specs (sous-commandes, options,
//   valeurs, fichiers et dossiers). Chaque generator dynamique y est remplacé par
//   son chemin dans la spec d'origine.
// - `modules.json.z` : le code JavaScript des specs qui ont des generators.
//   EasyTab l'exécute dans son moteur JS embarqué pour lancer ces generators.

import { readFileSync, readdirSync, writeFileSync } from "node:fs";
import { deflateSync } from "node:zlib";
import { join, resolve } from "node:path";
import { pathToFileURL } from "node:url";

// Fichiers du paquet qui ne sont pas des specs de commande.
const SKIP = new Set(["-", "index"]);

const TEMPLATES = new Set(["filepaths", "folders"]);

// Specs dont les suggestions viennent d'une fonction JS qu'on remplace par un
// équivalent déclaratif.
const OVERRIDES = {
  // Le generator de `cd` liste les dossiers.
  cd: (spec) => {
    spec.args[0].templates = ["folders"];
    delete spec.args[0].generators;
  },
};

const toArray = (value) => (value === undefined ? [] : Array.isArray(value) ? value : [value]);
const names = (value) => toArray(value).filter((n) => typeof n === "string" && n.length > 0);
const text = (value) => (typeof value === "string" && value.length > 0 ? value : undefined);
const insertValue = (value) =>
  typeof value === "string" ? value.replace(/\{cursor(?:\|\d+)?\}/g, "") : undefined;

function compact(object) {
  for (const key of Object.keys(object)) {
    const value = object[key];
    if (value === undefined || value === false || (Array.isArray(value) && value.length === 0)) {
      delete object[key];
    }
  }
  return object;
}

function suggestion(raw) {
  if (typeof raw === "string") return { names: [raw] };
  if (!raw || typeof raw !== "object") return undefined;
  const result = compact({
    names: names(raw.name),
    description: text(raw.description),
    insert: insertValue(raw.insertValue),
    hidden: raw.hidden === true,
  });
  return result.names ? result : undefined;
}

// Generator calculé par du JavaScript : on garde son chemin dans la spec, et ce
// qui sert à filtrer ses résultats quand c'est une simple chaîne.
function generator(raw, path) {
  if (!raw || typeof raw !== "object" || !(raw.script || raw.custom)) return undefined;
  return compact({
    path,
    trigger: text(raw.trigger),
    query_term: text(raw.getQueryTerm),
  });
}

function arg(raw, path) {
  if (!raw || typeof raw !== "object") return undefined;
  const templates = new Set(toArray(raw.template).filter((t) => TEMPLATES.has(t)));
  for (const generator of toArray(raw.generators)) {
    for (const t of toArray(generator?.template)) if (TEMPLATES.has(t)) templates.add(t);
  }
  const generators = Array.isArray(raw.generators)
    ? raw.generators.map((g, i) => generator(g, [...path, "generators", i]))
    : [generator(raw.generators, [...path, "generators"])];
  return compact({
    name: text(raw.name),
    description: text(raw.description),
    optional: raw.isOptional === true,
    variadic: raw.isVariadic === true,
    is_command: raw.isCommand === true,
    suggestions: toArray(raw.suggestions).map(suggestion).filter(Boolean),
    templates: [...templates],
    generators: generators.filter(Boolean),
  });
}

// Chemin d'un élément de `raw[key]`, qui peut être un tableau ou un objet seul.
const items = (raw, key, path, convert) =>
  Array.isArray(raw[key])
    ? raw[key].map((item, i) => convert(item, [...path, key, i]))
    : toArray(raw[key]).map((item) => convert(item, [...path, key]));

function option(raw, path) {
  if (!raw || typeof raw !== "object") return undefined;
  const result = compact({
    names: names(raw.name),
    description: text(raw.description),
    args: items(raw, "args", path, arg).filter(Boolean),
    persistent: raw.isPersistent === true,
    repeatable: raw.isRepeatable === true || typeof raw.isRepeatable === "number",
    requires_equals: raw.requiresEquals === true || raw.requiresSeparator === true,
    hidden: raw.hidden === true,
    insert: insertValue(raw.insertValue),
  });
  return result.names ? result : undefined;
}

function command(raw, path) {
  if (!raw || typeof raw !== "object") return undefined;
  const result = compact({
    names: names(raw.name),
    description: text(raw.description),
    subcommands: items(raw, "subcommands", path, command).filter(Boolean),
    options: items(raw, "options", path, option).filter(Boolean),
    args: items(raw, "args", path, arg).filter(Boolean),
    hidden: raw.hidden === true,
    insert: insertValue(raw.insertValue),
  });
  return result.names ? result : undefined;
}

const hasGenerators = (spec) =>
  JSON.stringify(spec).includes('"generators":');

const root = resolve(process.argv[2] ?? "");
const out = resolve(process.argv[3] ?? "specs");
const { version } = JSON.parse(readFileSync(join(root, "package.json"), "utf8"));
const specs = [];
const modules = {};
const commands = readdirSync(join(root, "build"))
  .filter((file) => file.endsWith(".js"))
  .map((file) => file.slice(0, -3))
  .filter((name) => !SKIP.has(name))
  .sort();
for (const name of commands) {
  const file = join(root, "build", `${name}.js`);
  const module = await import(pathToFileURL(file).href);
  const spec = command(module.default, []);
  if (!spec) {
    console.error(`ignorée (spec dynamique) : ${name}`);
    continue;
  }
  OVERRIDES[name]?.(spec);
  if (hasGenerators(spec)) {
    spec.module = name;
    modules[name] = readFileSync(file, "utf8");
  }
  specs.push(spec);
}
const source = `@withfig/autocomplete@${version}`;
const write = (file, value) =>
  writeFileSync(join(out, file), deflateSync(JSON.stringify(value), { level: 9 }));
write("specs.json.z", { source, specs });
write("modules.json.z", { source, modules });
console.error(`${specs.length} specs exportées, ${Object.keys(modules).length} avec generators`);
