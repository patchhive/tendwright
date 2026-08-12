#!/usr/bin/env node

import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";

const registryDir = "services/patchhive-backend/registry/products";
const specialistRoutesSource = readFileSync(
  "crates/patchhive-product-core/src/specialist_routes.rs",
  "utf8",
);
const standardSpecialistRoutes = stringArray(
  specialistRoutesSource,
  /pub const STANDARD_SPECIALIST_ROUTE_PATHS:[^=]+\=\s*&\[(.*?)\];/s,
  "STANDARD_SPECIALIST_ROUTE_PATHS",
);
let failed = false;

for (const manifestName of readdirSync(registryDir).filter(name => name.endsWith(".toml")).sort()) {
  const product = manifestName.slice(0, -".toml".length);
  const manifest = readFileSync(join(registryDir, manifestName), "utf8");
  const prefix = requiredMatch(manifest, /^route_prefix = "([^"]+)"/m, `${product} route_prefix`);
  const claimed = new Set();

  for (const match of manifest.matchAll(/^path = "([^"]+)"/gm)) {
    const path = match[1];
    if (!path.startsWith(prefix)) {
      report(product, `manifest route ${path} is outside ${prefix}`);
      continue;
    }
    claimed.add(path.slice(prefix.length) || "/");
  }

  const implemented = new Set();
  let usesStandardSpecialistRoutes = false;
  for (const sourcePath of rustSources(join("products", product, "backend", "src"))) {
    const source = readFileSync(sourcePath, "utf8").split("#[cfg(test)]", 1)[0];
    usesStandardSpecialistRoutes ||= /standard_specialist_router\s*\(/s.test(source);
    for (const match of source.matchAll(/\.route\(\s*"([^"]+)"/gs)) {
      implemented.add(match[1]);
    }
  }
  if (usesStandardSpecialistRoutes) {
    for (const path of standardSpecialistRoutes) implemented.add(path);
  }

  for (const path of difference(implemented, claimed)) {
    report(product, `router path ${path} is missing from the manifest`);
  }
  for (const path of difference(claimed, implemented)) {
    report(product, `manifest path ${path} is not implemented by the router`);
  }
}

if (failed) process.exit(1);

function rustSources(directory) {
  return readdirSync(directory, { withFileTypes: true }).flatMap(entry => {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) return rustSources(path);
    return entry.name.endsWith(".rs") ? [path] : [];
  });
}

function requiredMatch(source, pattern, label) {
  const match = source.match(pattern);
  if (!match) throw new Error(`Missing ${label}`);
  return match[1];
}

function stringArray(source, pattern, label) {
  const body = requiredMatch(source, pattern, label);
  const values = [...body.matchAll(/"([^"]+)"/g)].map(match => match[1]);
  if (values.length === 0) throw new Error(`${label} must not be empty`);
  return values;
}

function difference(left, right) {
  return [...left].filter(value => !right.has(value)).sort();
}

function report(product, message) {
  failed = true;
  console.error(`manifest routes: ${product}: ${message}`);
}
