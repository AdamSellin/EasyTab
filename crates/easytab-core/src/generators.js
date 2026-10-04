// Exécution des generators Fig dans le moteur JS d'EasyTab.
//
// Rust fournit `__easytab_exec(commande, arguments, dossier)`, qui lance une
// commande et renvoie `{stdout, stderr, status}` en JSON, et charge l'objet
// exporté par chaque spec dans `__easytab_modules`.

globalThis.__easytab_modules = {};

const quiet = () => {};
globalThis.console = { log: quiet, info: quiet, warn: quiet, error: quiet, debug: quiet };

// QuickJS n'a pas `Intl` : version minimale de ce qu'utilisent les specs.
globalThis.Intl ??= {
  NumberFormat: class {
    format(n) {
      if (n >= 1e9) return `${+(n / 1e9).toPrecision(3)}B`;
      if (n >= 1e6) return `${+(n / 1e6).toPrecision(3)}M`;
      if (n >= 1e3) return `${+(n / 1e3).toPrecision(3)}K`;
      return String(n);
    }
  },
  Collator: class {
    compare(a, b) {
      return a < b ? -1 : a > b ? 1 : 0;
    }
  },
};

globalThis.__easytab_run = async (module, path, tokens, cwd, env) => {
  let generator = __easytab_modules[module];
  for (const key of JSON.parse(path)) generator = generator[key];

  const exec = (command, args, dir) =>
    JSON.parse(__easytab_exec(command, args ?? [], dir ?? cwd));
  // `executeShellCommand` des generators `custom` : ancienne forme (une ligne
  // de shell, renvoie stdout) ou nouvelle (`{command, args, cwd}`).
  const executeShellCommand = async (input) =>
    typeof input === "string"
      ? exec("bash", ["-c", input]).stdout
      : exec(input.command, input.args, input.cwd);
  const context = {
    currentWorkingDirectory: cwd,
    environmentVariables: env,
    currentProcess: tokens[0] ?? "",
    searchTerm: tokens[tokens.length - 1] ?? "",
    sshPrefix: "",
    isDangerous: false,
  };

  let result = [];
  if (typeof generator.custom === "function") {
    result = await generator.custom(tokens, executeShellCommand, context);
  } else if (generator.script) {
    const script =
      typeof generator.script === "function" ? generator.script(tokens) : generator.script;
    let out;
    if (typeof script === "string") out = exec("bash", ["-c", script]).stdout;
    else if (Array.isArray(script) && script.length > 0)
      out = exec(script[0], script.slice(1)).stdout;
    else if (script && script.command) out = exec(script.command, script.args, script.cwd).stdout;
    if (out === undefined) result = [];
    else if (typeof generator.postProcess === "function")
      result = generator.postProcess(out, tokens);
    else if (generator.splitOn) result = out.split(generator.splitOn);
  }

  const items = [];
  for (const raw of result ?? []) {
    const suggestion = typeof raw === "string" ? { name: raw } : raw;
    if (!suggestion || typeof suggestion !== "object" || suggestion.hidden) continue;
    const names = (Array.isArray(suggestion.name) ? suggestion.name : [suggestion.name]).filter(
      (name) => typeof name === "string" && name.length > 0,
    );
    if (names.length === 0) continue;
    items.push({
      names,
      insert: typeof suggestion.insertValue === "string" ? suggestion.insertValue : undefined,
      description:
        typeof suggestion.description === "string" && suggestion.description.length > 0
          ? suggestion.description
          : undefined,
      priority: typeof suggestion.priority === "number" ? suggestion.priority : 50,
    });
  }
  return JSON.stringify(items);
};
