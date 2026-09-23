# JevLint for VS Code

This extension runs one `jevlint watch` process for every `.jevlintrc.jsonc`
found in a trusted workspace. It converts the versioned JSON Lines stream into
native VS Code diagnostics.

- Error and warning severities appear in the Problems panel and editor.
- Line-detected findings underline each complete reported line range.
- File-wide findings use the first line, which gives VS Code a conventional
  location for navigation and file-level decoration.
- Multi-root workspaces and nested project configurations are supported.
- Versioned snapshots prevent stale findings and preserve diagnostics that were
  unaffected by the current pass.
- Single-file passes replace only that file's diagnostics, so unrelated Problems
  entries do not flash during edits.
- Diagnostic messages come from the rule's Markdown message section, and the
  diagnostic code links to the originating `.md` rule file.
- Watch processes restart with bounded exponential backoff after unexpected
  exits.

## Install

Install the `jevlint` CLI first from the repository root:

```sh
./install-to-path.sh
```

Then install the extension:

```sh
cd ide-extension
./install.sh
```

The installer uses pnpm's frozen lockfile, runs type checking, Biome, formatting
checks, and tests, builds a bundled CommonJS extension, creates
`jevlint-vscode.vsix`, and installs it with the VS Code CLI. Set `CODE_BIN` to
another compatible command, such as `code-insiders`, when needed.

For development:

```sh
pnpm install
pnpm check
pnpm build
pnpm watch
```

## Configuration

The extension searches every workspace folder for `.jevlintrc.jsonc`. It contributes the Rust-derived JSON Schema to VS Code for validation and completion, without writing workspace settings. JevLint
paths remain relative to the folder containing that file, exactly as they are in
the CLI.

The executable defaults to `jevlint` on the extension host's `PATH`, with an
automatic fallback to `~/.local/bin/jevlint`. Override it with an absolute path
for any other installation location:

```json
{
  "jevlint.executablePath": "/Users/me/.local/bin/jevlint"
}
```

The CLI checks `TYPESAFE_API_KEY` first, then the config's
`typesafe_api_key_file`, then `~/.config/jevlint/typesafe-api-key`. Key files
contain only the raw key. They are read on each watch pass, so adding or
replacing one does not require quitting or reloading VS Code. Saving a relevant
file triggers a pass; **JevLint: Restart Watchers** is available when an
immediate retry is useful. For remote SSH, containers, or Codespaces, install
and configure the CLI in the remote environment because this is a workspace
extension.

Use **JevLint: Show Output** for process errors and precondition output.
Set `jevlint.trace.server` to `messages` or `verbose` for protocol debugging.

Workspace Trust is respected: no project command or JevLint process starts in an
untrusted workspace.
