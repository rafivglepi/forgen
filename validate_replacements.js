#!/usr/bin/env node
/**
 * validate_replacements.js
 *
 * Reads the replacement JSON files produced by `cargo forgen` from
 * target/.forgen/ and shows what each affected source file would look like
 * after the replacements are applied — without touching any source files.
 *
 * Usage
 * -----
 *   node validate_replacements.js                  # process every .json in target/.forgen/
 *   node validate_replacements.js test/src/lib.rs  # process a single source file
 *   node validate_replacements.js --diff           # show unified diff instead of full file
 *
 * Options
 *   --diff      Show a compact unified diff instead of the full resulting file.
 *   --no-color  Disable ANSI colour output.
 */

"use strict";

const fs = require("fs");
const path = require("path");

// ---------------------------------------------------------------------------
// CLI flags
// ---------------------------------------------------------------------------

const args = process.argv.slice(2);
const DIFF = args.includes("--diff");
const NO_COLOR = args.includes("--no-color") || !process.stdout.isTTY;
const targets = args.filter((a) => !a.startsWith("--"));

// ---------------------------------------------------------------------------
// Colour helpers
// ---------------------------------------------------------------------------

const c = {
  reset: NO_COLOR ? "" : "\x1b[0m",
  bold: NO_COLOR ? "" : "\x1b[1m",
  dim: NO_COLOR ? "" : "\x1b[2m",
  green: NO_COLOR ? "" : "\x1b[32m",
  red: NO_COLOR ? "" : "\x1b[31m",
  cyan: NO_COLOR ? "" : "\x1b[36m",
  yellow: NO_COLOR ? "" : "\x1b[33m",
};

function header(text) {
  const line = "─".repeat(Math.max(0, 60 - text.length - 2));
  return `\n${c.bold}${c.cyan}── ${text} ${line}${c.reset}`;
}

// ---------------------------------------------------------------------------
// Workspace root detection
// ---------------------------------------------------------------------------

/**
 * Walk up the directory tree from `startDir` and return the first directory
 * that contains a Cargo.toml with a `[workspace]` section.
 * Returns `null` if none is found.
 */
function findWorkspaceRoot(startDir) {
  let dir = path.resolve(startDir);
  while (true) {
    const toml = path.join(dir, "Cargo.toml");
    if (fs.existsSync(toml)) {
      try {
        const content = fs.readFileSync(toml, "utf8");
        if (content.includes("[workspace]")) return dir;
      } catch (_) {
        /* ignore */
      }
    }
    const parent = path.dirname(dir);
    if (parent === dir) return null; // reached filesystem root
    dir = parent;
  }
}

// ---------------------------------------------------------------------------
// Replacement application
// ---------------------------------------------------------------------------

/**
 * Find the byte offset of the nth occurrence of `needle` in `source`.
 *
 * Occurrences are zero-based and count overlapping matches.
 * Empty-string matches are counted at every UTF-8 character boundary.
 */
function findOccurrenceStart(source, needle, index) {
  if (!Number.isInteger(index) || index < 0) return null;

  if (needle === "") {
    let seen = 0;
    for (let i = 0; i <= source.length; i++) {
      if (i === 0 || i === source.length || isCharBoundary(source, i)) {
        if (seen === index) return i;
        seen++;
      }
    }
    return null;
  }

  let seen = 0;
  for (let i = 0; i + needle.length <= source.length; i++) {
    if (!isCharBoundary(source, i)) continue;
    if (source.startsWith(needle, i)) {
      if (seen === index) return i;
      seen++;
    }
  }

  return null;
}

/**
 * Return whether `offset` is a UTF-16 code unit boundary that is safe to use
 * as a string slice boundary in JS without splitting a surrogate pair.
 */
function isCharBoundary(source, offset) {
  if (offset <= 0 || offset >= source.length) return true;
  const prev = source.charCodeAt(offset - 1);
  const next = source.charCodeAt(offset);
  const prevIsHigh = prev >= 0xd800 && prev <= 0xdbff;
  const nextIsLow = next >= 0xdc00 && next <= 0xdfff;
  return !(prevIsHigh && nextIsLow);
}

/**
 * Resolve occurrence-based replacements into byte ranges within `source`.
 *
 * Each replacement object must have the shape:
 *   { index: number, old_text: string, new_text: string }
 */
function resolveReplacements(source, replacements) {
  const resolved = [];

  for (const rep of replacements) {
    const { index, old_text, new_text } = rep;

    if (typeof index !== "number" || index < 0 || !Number.isInteger(index)) {
      console.warn(
        `${c.yellow}  ⚠  Skipping replacement with invalid index: ${JSON.stringify(rep)}${c.reset}`,
      );
      continue;
    }

    if (typeof old_text !== "string" || typeof new_text !== "string") {
      console.warn(
        `${c.yellow}  ⚠  Skipping replacement with invalid text fields: ${JSON.stringify(rep)}${c.reset}`,
      );
      continue;
    }

    const start = findOccurrenceStart(source, old_text, index);
    if (start == null) {
      console.warn(
        `${c.yellow}  ⚠  Could not find occurrence #${index} of ${JSON.stringify(old_text)}${c.reset}`,
      );
      continue;
    }

    const end = start + old_text.length;
    resolved.push({ start, end, text: new_text, old_text, index });
  }

  return resolved;
}

/**
 * Apply an array of occurrence-based replacements to `source`.
 *
 * Resolved replacements are applied in descending order by start offset so that
 * later edits do not shift the positions of earlier ones.
 */
function applyReplacements(source, replacements) {
  const resolved = resolveReplacements(source, replacements);
  const sorted = [...resolved].sort((a, b) => b.start - a.start);

  let result = source;
  for (const rep of sorted) {
    const { start, end, text } = rep;

    if (start < 0 || end < start || end > result.length) {
      console.warn(
        `${c.yellow}  ⚠  Skipping out-of-bounds replacement ` +
          `[${start}..${end}] (file length ${result.length})${c.reset}`,
      );
      continue;
    }

    result = result.slice(0, start) + text + result.slice(end);
  }

  return result;
}

// ---------------------------------------------------------------------------
// Minimal unified diff
// ---------------------------------------------------------------------------

/**
 * Produce a very simple line-level unified diff between `before` and `after`.
 * Not a full Myers diff — good enough for visual inspection of small changes.
 */
function unifiedDiff(before, after, filePath) {
  const oldLines = before.split("\n");
  const newLines = after.split("\n");

  const lines = [];
  lines.push(`${c.bold}--- ${filePath} (original)${c.reset}`);
  lines.push(`${c.bold}+++ ${filePath} (with replacements)${c.reset}`);

  // Build a simple line-by-line comparison using a naive LCS.
  const lcs = buildLCS(oldLines, newLines);
  let oi = 0,
    ni = 0,
    li = 0;
  let hunkLines = [];
  let inHunk = false;

  function flushHunk() {
    if (hunkLines.length > 0) {
      lines.push(...hunkLines);
      hunkLines = [];
    }
    inHunk = false;
  }

  while (oi < oldLines.length || ni < newLines.length) {
    if (
      li < lcs.length &&
      oi < oldLines.length &&
      ni < newLines.length &&
      oldLines[oi] === lcs[li] &&
      newLines[ni] === lcs[li]
    ) {
      // Context line — only show 2 lines of context around changes.
      if (inHunk) {
        hunkLines.push(`${c.dim} ${oldLines[oi]}${c.reset}`);
        if (
          hunkLines.filter((l) => l.startsWith(c.red) || l.startsWith(c.green))
            .length === 0
        ) {
          hunkLines = [];
        }
      }
      oi++;
      ni++;
      li++;
    } else if (
      ni < newLines.length &&
      (li >= lcs.length || newLines[ni] !== lcs[li])
    ) {
      inHunk = true;
      hunkLines.push(`${c.green}+${newLines[ni]}${c.reset}`);
      ni++;
    } else if (
      oi < oldLines.length &&
      (li >= lcs.length || oldLines[oi] !== lcs[li])
    ) {
      inHunk = true;
      hunkLines.push(`${c.red}-${oldLines[oi]}${c.reset}`);
      oi++;
    } else {
      break;
    }
  }

  flushHunk();
  return lines.join("\n");
}

/** Build the Longest Common Subsequence of two string arrays. */
function buildLCS(a, b) {
  const m = Math.min(a.length, 200); // cap to avoid O(n²) on huge files
  const n = Math.min(b.length, 200);
  const dp = Array.from({ length: m + 1 }, () => new Array(n + 1).fill(0));

  for (let i = 1; i <= m; i++) {
    for (let j = 1; j <= n; j++) {
      dp[i][j] =
        a[i - 1] === b[j - 1]
          ? dp[i - 1][j - 1] + 1
          : Math.max(dp[i - 1][j], dp[i][j - 1]);
    }
  }

  // Backtrack.
  const result = [];
  let i = m,
    j = n;
  while (i > 0 && j > 0) {
    if (a[i - 1] === b[j - 1]) {
      result.push(a[i - 1]);
      i--;
      j--;
    } else if (dp[i - 1][j] > dp[i][j - 1]) i--;
    else j--;
  }
  return result.reverse();
}

// ---------------------------------------------------------------------------
// Per-file processing
// ---------------------------------------------------------------------------

/**
 * Process a single source file: find its corresponding .json, apply
 * replacements, and display the result (or diff).
 */
function processFile(sourceFile, workspaceRoot) {
  const absSource = path.resolve(sourceFile);

  if (!fs.existsSync(absSource)) {
    console.error(`${c.red}❌  Source file not found: ${absSource}${c.reset}`);
    return false;
  }

  // Normalise to forward slashes for display / JSON lookup.
  const relPath = path.relative(workspaceRoot, absSource).replace(/\\/g, "/");
  const jsonPath = path.join(
    workspaceRoot,
    "target",
    ".forgen",
    relPath + ".json",
  );

  if (!fs.existsSync(jsonPath)) {
    console.log(
      `${c.yellow}⚠  No replacement file for ${relPath}${c.reset}\n` +
        `   (expected: target/.forgen/${relPath}.json)\n` +
        `   Run ${c.bold}cargo forgen${c.reset} first.`,
    );
    return false;
  }

  let replacements;
  try {
    replacements = JSON.parse(fs.readFileSync(jsonPath, "utf8"));
  } catch (e) {
    console.error(
      `${c.red}❌  Failed to parse ${jsonPath}: ${e.message}${c.reset}`,
    );
    return false;
  }

  if (!Array.isArray(replacements)) {
    console.error(
      `${c.red}❌  ${jsonPath} does not contain a JSON array${c.reset}`,
    );
    return false;
  }

  if (
    replacements.some(
      (rep) =>
        !rep ||
        typeof rep !== "object" ||
        typeof rep.index !== "number" ||
        typeof rep.old_text !== "string" ||
        typeof rep.new_text !== "string",
    )
  ) {
    console.error(
      `${c.red}❌  ${jsonPath} does not contain occurrence-based replacements ` +
        `{ index, old_text, new_text }${c.reset}`,
    );
    return false;
  }

  const source = fs.readFileSync(absSource, "utf8");

  console.log(header(relPath));
  console.log(
    `${c.bold}${replacements.length}${c.reset} replacement(s) from ` +
      `${c.dim}target/.forgen/${relPath}.json${c.reset}`,
  );
  console.log();

  // Print replacement summary table.
  const resolved = resolveReplacements(source, replacements);
  for (let i = 0; i < replacements.length; i++) {
    const rep = replacements[i];
    const resolvedRep = resolved[i];

    const kind = !resolvedRep
      ? "invalid"
      : resolvedRep.start === resolvedRep.end
        ? "insert"
        : resolvedRep.text === ""
          ? "delete"
          : "replace";

    const preview = String(rep.new_text ?? "")
      .replace(/\n/g, "↵")
      .replace(/\t/g, "→")
      .slice(0, 60);

    const kindColor =
      kind === "insert"
        ? c.green
        : kind === "delete"
          ? c.red
          : kind === "invalid"
            ? c.yellow
            : c.yellow;

    const location = resolvedRep
      ? `@ ${c.bold}[${resolvedRep.start}..${resolvedRep.end}]${c.reset}`
      : `${c.yellow}(unresolved)${c.reset}`;

    console.log(
      `  ${c.dim}[${String(i + 1).padStart(2)}]${c.reset} ` +
        `${kindColor}${kind.padEnd(7)}${c.reset} ` +
        `#${rep.index} ${c.dim}${JSON.stringify(rep.old_text)}${c.reset} ${location}` +
        (rep.new_text
          ? `  ${c.dim}"${preview}${rep.new_text.length > 60 ? "…" : ""}"${c.reset}`
          : ""),
    );
  }

  console.log();

  const result = applyReplacements(source, replacements);

  if (DIFF) {
    console.log(unifiedDiff(source, result, relPath));
  } else {
    console.log(`${c.bold}Result:${c.reset}`);
    console.log("─".repeat(60));
    // Print with line numbers.
    const lines = result.split("\n");
    const width = String(lines.length).length;
    lines.forEach((line, idx) => {
      const num = String(idx + 1).padStart(width);
      console.log(`${c.dim}${num} │${c.reset} ${line}`);
    });
    console.log("─".repeat(60));
  }

  return true;
}

// ---------------------------------------------------------------------------
// Directory walker — process all .json files under target/.forgen/
// ---------------------------------------------------------------------------

function walkDir(dir, callback) {
  if (!fs.existsSync(dir)) return;
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      walkDir(full, callback);
    } else if (entry.isFile() && entry.name.endsWith(".json")) {
      callback(full);
    }
  }
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

const workspaceRoot = findWorkspaceRoot(process.cwd());

if (!workspaceRoot) {
  console.error(
    `${c.red}❌  Could not find a workspace root (Cargo.toml with [workspace]).${c.reset}\n` +
      `   Run this script from inside your Cargo workspace.`,
  );
  process.exit(1);
}

console.log(`${c.bold}🏠 Workspace root:${c.reset} ${workspaceRoot}`);
if (DIFF) console.log(`${c.dim}(showing unified diff)${c.reset}`);

let processed = 0;
let ok = 0;

if (targets.length > 0) {
  // Explicit file(s) on the command line.
  for (const t of targets) {
    processed++;
    if (processFile(t, workspaceRoot)) ok++;
  }
} else {
  // No explicit target → walk every .json file in target/.forgen/.
  const forgenDir = path.join(workspaceRoot, "target", ".forgen");

  if (!fs.existsSync(forgenDir)) {
    console.error(
      `\n${c.red}❌  target/.forgen/ not found.${c.reset}\n` +
        `   Run ${c.bold}cargo forgen${c.reset} first to generate replacement files.`,
    );
    process.exit(1);
  }

  walkDir(forgenDir, (jsonFile) => {
    // Reconstruct the source path by stripping the forgenDir prefix and
    // the trailing ".json" suffix.
    const relJson = path.relative(forgenDir, jsonFile);
    const relSource = relJson.replace(/\.json$/, "");
    const sourceFile = path.join(workspaceRoot, relSource);

    if (fs.existsSync(sourceFile)) {
      processed++;
      if (processFile(sourceFile, workspaceRoot)) ok++;
    } else {
      console.warn(
        `${c.yellow}⚠  JSON found but source missing: ${relSource}${c.reset}`,
      );
    }
  });
}

console.log(
  `\n${ok === processed ? c.green : c.yellow}` +
    `✔  Processed ${ok}/${processed} file(s).${c.reset}\n`,
);

if (ok === 0 && processed === 0) {
  console.log(
    `${c.dim}No replacement files found. Run ${c.bold}cargo forgen${c.reset}${c.dim} first.${c.reset}`,
  );
}
