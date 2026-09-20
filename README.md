# jevlint

`jevlint` is a concurrent semantic code linter backed by Jev. It evaluates every selected file against Markdown rules, caches each rule result independently, and can make a second cached pass that identifies the lines involved in each violation.

## Set up

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

## Cache and execution model

The cache is a transactional redb database under `.jevlint/cache.redb`. Verdict keys include the normalized relative path, file content, individual rule content, system prompt, requested model, response schema, and tool namespace. Line-location entries add the line number and use a separate schema namespace. Writes are atomic; reads and writes are batched per file.

Files are scheduled with bounded Tokio concurrency. One verdict request contains every uncached rule for a file. Only failed rules enter line detection, where every `(rule, line)` question is independently cached and requests are bounded by `max_questions_per_request`. Adjacent positive lines are printed as one region.

File discovery honors Git ignore files. Watching, debouncing, LSP diagnostics, and tool-specific precondition adapters are deliberately outside this first CLI pass; the reusable engine and provider/precondition traits are the integration boundaries for them.
