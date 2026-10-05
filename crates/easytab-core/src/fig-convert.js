// Conversion d'une spec Fig (objet JavaScript) vers le format déclaratif lu par
// easytab-core (voir `spec.rs`).
//
// Partagé par l'import des specs (`tools/import-fig-specs.mjs`, sous Node) et
// par le moteur JS embarqué, qui convertit les specs produites par
// `generateSpec`. Script classique, sans `import` ni `export` : il définit
// `globalThis.__easytab_convert(spec, options)`.
//
// Options :
// - `module` : module JS des generators de la spec convertie. Seulement pour une
//   spec produite par `generateSpec` : ses generators gardent ce nom, parce que
//   leur chemin se lit dans l'objet généré et non dans le module de la spec.

(() => {
  const TEMPLATES = new Set(["filepaths", "folders"]);

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

  // Chemin d'un élément de `raw[key]`, qui peut être un tableau ou un objet seul.
  const items = (raw, key, path, convert) =>
    Array.isArray(raw[key])
      ? raw[key].map((item, i) => convert(item, [...path, key, i]))
      : toArray(raw[key]).map((item) => convert(item, [...path, key]));

  function converter(options) {
    const suggestion = (raw) => {
      if (typeof raw === "string") return { names: [raw] };
      if (!raw || typeof raw !== "object") return undefined;
      const result = compact({
        names: names(raw.name),
        description: text(raw.description),
        insert: insertValue(raw.insertValue),
        hidden: raw.hidden === true,
      });
      return result.names ? result : undefined;
    };

    // Generator calculé par du JavaScript : on garde son chemin dans la spec, et
    // ce qui sert à filtrer ses résultats quand c'est une simple chaîne.
    const generator = (raw, path) => {
      if (!raw || typeof raw !== "object" || !(raw.script || raw.custom)) return undefined;
      return compact({
        path,
        module: options.module,
        trigger: text(raw.trigger),
        query_term: text(raw.getQueryTerm),
      });
    };

    const arg = (raw, path) => {
      if (!raw || typeof raw !== "object") return undefined;
      const templates = new Set(toArray(raw.template).filter((t) => TEMPLATES.has(t)));
      for (const generator of toArray(raw.generators)) {
        for (const t of toArray(generator?.template)) if (TEMPLATES.has(t)) templates.add(t);
      }
      const generators = Array.isArray(raw.generators)
        ? raw.generators.map((g, i) => generator(g, [...path, "generators", i]))
        : [generator(raw.generators, [...path, "generators"])];
      // `loadSpec` sur un argument : la suite de la ligne se complète avec une
      // autre spec (par son nom) ou avec la spec donnée sur place. La forme
      // fonction (`bun create <modèle>`) n'est pas reprise.
      const inline =
        raw.loadSpec && typeof raw.loadSpec === "object"
          ? command({ name: "spec", ...raw.loadSpec }, [...path, "loadSpec"])
          : undefined;
      return compact({
        name: text(raw.name),
        description: text(raw.description),
        optional: raw.isOptional === true,
        variadic: raw.isVariadic === true,
        is_command: raw.isCommand === true,
        suggestions: toArray(raw.suggestions).map(suggestion).filter(Boolean),
        templates: [...templates],
        generators: generators.filter(Boolean),
        load: text(raw.loadSpec),
        spec: inline,
      });
    };

    const option = (raw, path) => {
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
    };

    const command = (raw, path) => {
      if (!raw || typeof raw !== "object") return undefined;
      const result = compact({
        names: names(raw.name),
        description: text(raw.description),
        subcommands: items(raw, "subcommands", path, command).filter(Boolean),
        options: items(raw, "options", path, option).filter(Boolean),
        args: items(raw, "args", path, arg).filter(Boolean),
        hidden: raw.hidden === true,
        insert: insertValue(raw.insertValue),
        // Le contenu vient d'une autre spec (`php/bin-console`) : lue à la demande.
        load: text(raw.loadSpec),
      });
      // Sous-commandes et options calculées par `generateSpec` (chemin de la
      // fonction dans la spec ; `[]` pour la spec elle-même). Pas dans une spec
      // déjà générée : son objet n'est pas un module qu'on sait recharger.
      if (typeof raw.generateSpec === "function" && options.module === undefined) {
        result.generate = path;
      }
      return result.names ? result : undefined;
    };

    return command;
  }

  globalThis.__easytab_convert = (spec, options = {}) => converter(options)(spec, []);
})();
