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

```sh
export TYPESAFE_API_KEY=...
cargo run --release -- --dry-run
cargo run --release
```

The optional project precondition runs once before any Jev requests. A nonzero exit marks every selected file as skipped and exits with status 1. This conservative batch behavior works with commands such as `cargo check`, `tsc --noEmit`, Biome, or ESLint without repeatedly invoking them per file.

Exit status is 0 when every rule passes, 1 for lint violations or a failed precondition, and 2 for configuration, I/O, cache, or API errors.

Ready-to-copy configurations for TypeScript with TSC and Biome, and Python with
Pyright and Ruff, are available under [`example-configs/`](example-configs/).

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

Each run emits `run_started`, followed by one complete `snapshot`. A consumer
should replace all previous diagnostics when a snapshot arrives. Sequence
numbers associate the two messages and allow consumers to discard stale data.
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

File discovery and watching honor the configured matchers and Git ignore files.
LSP diagnostics and tool-specific per-file precondition adapters remain outside
this pass; the reusable engine and provider/precondition traits are their
integration boundaries.
