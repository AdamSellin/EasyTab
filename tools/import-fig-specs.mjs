// Convertit les specs Fig (@withfig/autocomplete, licence MIT) en JSON statique
// lisible par easytab-core.
//
// Usage : node tools/import-fig-specs.mjs <dossier du paquet @withfig/autocomplete> > specs/specs.json
//
// Seule la partie déclarative des specs est gardée : les fonctions (generators
// dynamiques, versions, custom) sont ignorées pour l'instant, sauf les generators
// qui se contentent de demander des fichiers ou des dossiers.

import { readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { pathToFileURL } from "node:url";

// Commandes embarquées dans EasyTab. Garder la liste triée.
const COMMANDS = [
  "apt", "brew", "cargo", "cat", "cd", "chmod", "code", "cp", "curl", "docker",
  "docker-compose", "find", "gh", "git", "go", "grep", "head", "kill", "kubectl",
  "less", "ln", "ls", "make", "man", "mkdir", "mv", "node", "npm", "npx", "pip",
  "pip3", "pnpm", "python", "python3", "rm", "rmdir", "rustup", "scp", "ssh",
  "sudo", "systemctl", "tail", "tar", "touch", "vim", "wget", "yarn",
];

const TEMPLATES = new Set(["filepaths", "folders"]);

// Specs dont les suggestions viennent d'une fonction JS qu'on remplace par un
// équivalent déclaratif.
const OVERRIDES = {
  // Le generator de `cd` liste les dossiers.
  cd: (spec) => {
    spec.args[0].templates = ["folders"];
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

function arg(raw) {
  if (!raw || typeof raw !== "object") return undefined;
  const templates = new Set(toArray(raw.template).filter((t) => TEMPLATES.has(t)));
  for (const generator of toArray(raw.generators)) {
    for (const t of toArray(generator?.template)) if (TEMPLATES.has(t)) templates.add(t);
  }
  return compact({
    name: text(raw.name),
    description: text(raw.description),
    optional: raw.isOptional === true,
    variadic: raw.isVariadic === true,
    is_command: raw.isCommand === true,
    suggestions: toArray(raw.suggestions).map(suggestion).filter(Boolean),
    templates: [...templates],
  });
}

function option(raw) {
  if (!raw || typeof raw !== "object") return undefined;
  const result = compact({
    names: names(raw.name),
    description: text(raw.description),
    args: toArray(raw.args).map(arg).filter(Boolean),
    persistent: raw.isPersistent === true,
    repeatable: raw.isRepeatable === true || typeof raw.isRepeatable === "number",
    requires_equals: raw.requiresEquals === true || raw.requiresSeparator === true,
    hidden: raw.hidden === true,
    insert: insertValue(raw.insertValue),
  });
  return result.names ? result : undefined;
}

function command(raw) {
  if (!raw || typeof raw !== "object") return undefined;
  const result = compact({
    names: names(raw.name),
    description: text(raw.description),
    subcommands: toArray(raw.subcommands).map(command).filter(Boolean),
    options: toArray(raw.options).map(option).filter(Boolean),
    args: toArray(raw.args).map(arg).filter(Boolean),
    hidden: raw.hidden === true,
    insert: insertValue(raw.insertValue),
  });
  return result.names ? result : undefined;
}

const root = resolve(process.argv[2] ?? "");
const { version } = JSON.parse(readFileSync(join(root, "package.json"), "utf8"));
const specs = [];
for (const name of COMMANDS) {
  const module = await import(pathToFileURL(join(root, "build", `${name}.js`)).href);
  const spec = command(module.default);
  if (!spec) {
    console.error(`ignorée (spec dynamique) : ${name}`);
    continue;
  }
  OVERRIDES[name]?.(spec);
  specs.push(spec);
}
process.stdout.write(JSON.stringify({ source: `@withfig/autocomplete@${version}`, specs }));
process.stdout.write("\n");
console.error(`${specs.length} specs exportées`);
