# jevlint

`jevlint` is a concurrent semantic code linter backed by Jev. It evaluates every selected file against Markdown rules, caches each rule result independently, and can make a second cached pass that identifies the lines involved in each violation.

## Set up

Build an optimized binary and install it to `~/.local/bin`:

```sh
./install-to-path.sh
```

Set `JEVLINT_INSTALL_DIR` to override the destination. The installer uses an
atomic replacement and can safely be rerun after updating the source.

Copy `.jevlintrc.example.toml` to `.jevlintrc.toml`, create `.jevlint-system.md`, and put one rule in each `.jevlint-rules/*.md` file. Rule filenames are stable rule IDs.

Put the raw TypeSafe API key in `~/.config/jevlint/typesafe-api-key`, or set
`TYPESAFE_API_KEY`. The environment variable takes precedence. A project may
choose another file, relative to its config directory or absolute:

```toml
typesafe_api_key_file = "~/.config/jevlint/typesafe-api-key"
```

The key is read at the start of every run, including every watch pass, so it can
be added or replaced without restarting VS Code. Then verify file selection and
run the linter:

```sh
cargo run --release -- --dry-run
cargo run --release
```

Assign rules to file groups with override blocks. Patterns are gitignore-style
globs. A file receives the union of rules in every matching block, and
`excluded_files` removes it from that block only:

```toml
[[overrides]]
files = ["*.py", "**/*.py"]
rules = ["python-boundaries"]

[[overrides]]
files = ["*.ts", "*.tsx", "**/*.ts", "**/*.tsx"]
excluded_files = ["**/*.generated.ts"]
rules = ["typescript-boundaries", "shared-api-contracts"]
```

When any override exists, files matching no override are not linted. Without
overrides, every discovered file receives every rule for backward compatibility.

The optional project precondition runs once before any Jev requests. A nonzero exit marks every selected file as skipped and exits with status 1. This conservative batch behavior works with commands such as `cargo check`, `tsc --noEmit`, Biome, or ESLint without repeatedly invoking them per file.

Rules are errors by default. Configure warning-only rules by their Markdown
filename without the `.md` extension:

```toml
[rule_severity]
security-boundary = "error"
maintainability-note = "warning"
```

Unknown rule IDs and values other than `error` or `warning` are rejected.
Warnings appear in human and machine output but do not make a check exit with
status 1.

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

## VS Code extension

Install the bundled VS Code extension directly from this repository:

```sh
./ide-extension/install.sh
```

It discovers `.jevlintrc.toml` files, runs the watch protocol, and publishes
native error and warning diagnostics. See
[`ide-extension/README.md`](ide-extension/README.md) for development commands
and settings.

## Watch mode and editor integration

`jevlint watch` performs an initial run, watches the project with native
filesystem notifications, and reruns after a configurable quiet period:

```toml
[watch]
debounce_milliseconds = 300
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

Files are scheduled with bounded Tokio concurrency. One verdict request contains every uncached rule for a file. Only failed rules enter line detection, where every `(rule, line)` question is independently cached and requests are bounded by `max_questions_per_request`. Adjacent positive lines are printed as one region.

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
