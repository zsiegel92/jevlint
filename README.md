# jevlint

`jevlint` is a concurrent semantic code linter backed by Jev. It evaluates every selected file against Markdown rules, caches each rule result independently, and can make a second cached pass that identifies the lines involved in each violation.

## Set up

Build an optimized binary and install it to `~/.local/bin`:

```sh
./install-to-path.sh
```

Set `JEVLINT_INSTALL_DIR` to override the destination. The installer uses an
atomic replacement and can safely be rerun after updating the source.

Run `jevlint init` to create `.jevlintrc.jsonc`, a system prompt, and a starter Markdown rule. The config accepts `//` and `/* ... */` comments and trailing commas. Rename existing `.jevlintrc.json` files to `.jevlintrc.jsonc`. Rule filenames are stable rule IDs. The installer also places a generated JSON Schema at `~/.local/share/jevlint/jevlint.schema.json`; `init` adds a `$schema` URI when that installed file is available. The VS Code extension associates the schema with `.jevlintrc.jsonc` without changing workspace settings.

Put the raw TypeSafe API key in `~/.config/jevlint/typesafe-api-key`, or set
`TYPESAFE_API_KEY`. The environment variable takes precedence. A project may
choose another file, relative to its config directory or absolute:

```json
"typesafe_api_key_file": "~/.config/jevlint/typesafe-api-key"
```

The key is read at the start of every run, including every watch pass, so it can
be added or replaced without restarting VS Code. Then verify file selection and
run the linter:

```sh
cargo run --release -- --dry-run
cargo run --release
```

Every linted file and rule comes from a `rule_sets` entry. `match` contains glob patterns (including `*.ts` and `**/*.ts`), and a rule's number is its minimum verdict confidence. `exclude` removes files from that entry:

```json
"rule_sets": [
  {
    "match": ["*.py", "**/*.py"],
    "error": { "python-boundaries": 0.8 }
  },
  {
    "match": ["*.ts", "*.tsx", "**/*.ts", "**/*.tsx"],
    "exclude": ["**/*.generated.ts"],
    "error": { "typescript-boundaries": 0.9 },
    "warn": { "shared-api-contracts": 0.8 }
  }
]
```

A file receives the union of rules from its matching sets. If the same rule is
listed more than once, the later matching set determines its severity and threshold. Files
matching no rule set are not linted. There are no implicit rules or default file
patterns. The JSON Schema flags thresholds outside 0–1 inclusive. For callers
that bypass schema validation, the CLI still clamps values above 1 to 1 and
disables values below 0.

The optional project precondition runs once before any Jev requests. A nonzero exit marks every selected file as skipped and exits with status 1. This conservative batch behavior works with commands such as `cargo check`, `tsc --noEmit`, Biome, or ESLint without repeatedly invoking them per file.

Unknown enabled rule IDs are rejected.
Warnings appear in human and machine output but do not make a check exit with
status 1.

To suppress a reported line, include `jevlint-ignore` anywhere on that source
line. The final report omits that line from CLI output, JSON output, and watch
diagnostics. Other lines in the same finding remain visible. Findings without
line locations remain visible.

The rule's editor/CLI message is the Markdown before its first horizontal rule
(`---` or longer). Everything after that separator remains part of the full
instruction sent to Jev. With no separator, the whole rule is the message. Watch
diagnostics also carry the project-relative rule file path, which VS Code exposes
as a clickable diagnostic code.

Exit status is 0 when there are no error-severity violations, 1 for one or more
errors or a failed precondition, and 2 for configuration, I/O, cache, or API
errors.

Ready-to-copy configurations for TypeScript with TSC and Biome, and Python with
Pyright and Ruff, are available under [`example-configs/`](example-configs/).

## CLI smoke tests

Run the two six-file fixture projects explicitly with:

```sh
./smoke-tests/run.sh
```

To force every smoke pass to bypass cached results, run
`./smoke-tests/run.sh --force-fresh`. The default command deliberately checks
that its second pass reuses the cache. Both modes start from a new temporary
fixture copy, so neither uses cache files from previous smoke runs.

The command builds the local debug CLI and a separate smoke runner. It uses a
deterministic local Jev stand-in, so it needs no API key or network connection.
It checks clean and violating Python, TypeScript, and JSON files; exact rule IDs,
severities, and line regions; exit codes; precondition skips; a fully cached
second pass; and single-file cache invalidation after an edit. The runner
copies fixtures into a temporary directory, leaving the repository free of
smoke-test cache files. It is not part of `cargo test`.

Fixture configs are named `jevlint.smoke.jsonc`, not `.jevlintrc.jsonc`, so the
VS Code extension does not discover or watch them. The runner passes each config
explicitly via `--config`. One-shot checks also support `--json` for a typed
`RunReport` on stdout, with the usual 0/1/2 exit statuses.
For an ordinary one-shot CLI check, `jevlint --force-fresh` bypasses the existing
cache for that run without deleting or changing it. This is intentionally a CLI
flag rather than a config setting, so watch mode keeps caching normally.

## VS Code extension

Install the bundled VS Code extension directly from this repository:

```sh
./ide-extension/install.sh
```

It discovers `.jevlintrc.jsonc` files, runs the watch protocol, and publishes
native error and warning diagnostics. See
[`ide-extension/README.md`](ide-extension/README.md) for development commands
and settings.

## Watch mode and editor integration

`jevlint watch` performs an initial run, watches the project with native
filesystem notifications, and reruns after a configurable quiet period:

```json
"watch": { "debounce_milliseconds": 300 }
```

By default, stdout is a JSON Lines protocol intended to be consumed directly by
an editor extension:

```sh
jevlint watch
```

Each run emits `run_started`, followed by one complete `snapshot`. The snapshot's
`updated_paths` identifies exactly which files a streaming consumer should
replace, while its `diagnostics` still contains the complete current set for
file-based consumers. Sequence numbers associate the two messages and allow
consumers to discard stale data.
Operational failures are snapshots with `"status":"error"`, so a malformed
rule or temporary API failure clears stale diagnostics without terminating the
watcher.

The latest snapshot can additionally be written to disk. Replacement is atomic,
so readers never observe partial JSON:

```sh
jevlint watch --output .jevlint/diagnostics.json
```

Use `--no-stdout` with `--output` for a polling-only integration. Capturing
stdout is preferred for a VS Code extension because it avoids polling latency;
the file option is useful for simpler integrations and debugging. See
[`docs/watch-protocol.md`](docs/watch-protocol.md) for the versioned schema.

## Cache and execution model

The cache is a transactional redb database under `.jevlint/cache.redb`. Verdict keys include the normalized relative path, file content, individual rule content, system prompt, requested model, response schema, and tool namespace. Line-location entries add the line number and use a separate schema namespace. Writes are atomic; reads and writes are batched per file.

Files are scheduled with bounded Tokio concurrency. At most two Jev requests run at once across local `jevlint` processes, including both verdict and line detection requests. One verdict request contains every uncached rule for a file. Only failed rules enter line detection, where every `(rule, line)` question is independently cached and requests are bounded by `max_questions_per_request`. Adjacent positive lines are printed as one region.

Ordinary watch events analyze only the affected files and preserve all other
diagnostics. Changes to the config, system prompt, or a rule can affect the
whole project and therefore trigger a full cached pass. Watch mode uses native
filesystem events; `watch.debounce_milliseconds` controls how long it waits for
a quiet period while coalescing save bursts.

File discovery and watching honor the configured matchers and Git ignore files.
Rule-file edits are watched too; changing a rule invalidates only that rule's
cached verdicts and refreshes diagnostics for open files on the next pass.
LSP diagnostics and tool-specific per-file precondition adapters remain outside
this pass; the reusable engine and provider/precondition traits are their
integration boundaries.
