# Watch protocol

Watch mode writes one JSON object per line to stdout. Consumers must ignore
unknown object fields and object kinds so the protocol can grow compatibly.
`schema_version` is currently `1`.

## Run started

```json
{
  "schema_version": 1,
  "kind": "run_started",
  "sequence": 2,
  "trigger": "filesystem",
  "changed_paths": ["src/auth.ts"]
}
```

`trigger` is `initial` for the first run and `filesystem` thereafter. Changed
paths are project-relative and are informational: a run may inspect additional
files because rules, shared prompts, and preconditions have project-wide effects.

## Snapshot

```json
{
  "schema_version": 1,
  "kind": "snapshot",
  "sequence": 2,
  "root": "/workspace/project",
  "line_base": 1,
  "status": "violations",
  "full_update": false,
  "updated_paths": ["src/auth.ts"],
  "diagnostics": [
    {
      "path": "src/auth.ts",
      "rule_id": "no-unvalidated-boundary-cast",
      "rule_path": ".jevlint-rules/no-unvalidated-boundary-cast.md",
      "message": "# Validate boundary data\n\nExternal values must be validated before use.",
      "severity": "error",
      "confidence": 0.94,
      "regions": [{ "start_line": 42, "end_line": 47 }]
    }
  ],
  "stats": {
    "files_checked": 18,
    "rules": 2,
      "api_requests": 1,
      "cache_hits": 35,
      "violations": 1,
      "errors": 1,
      "warnings": 0
  },
  "precondition": null,
  "error": null
}
```

A snapshot contains the complete current diagnostic set. `updated_paths` is the
subset whose diagnostics became authoritative in this sequence; streaming
consumers should replace only those paths and leave every other editor
diagnostic untouched. This avoids workspace-wide diagnostic flicker after a
single-file edit. When `full_update` is true, consumers should replace the
entire diagnostic set; this is used for initial runs and project-wide rule,
prompt, or configuration changes. File-output consumers may simply replace
their full stored snapshot.

Paths, including `rule_path`, are relative to `root`. `message` comes from the
rule Markdown before its first horizontal rule (`---` or longer), or from the
whole file when it has no horizontal rule. Lines are inclusive and use
`line_base`; a diagnostic with no regions applies to the whole file.
`severity` is either `error` or `warning` and should be mapped to the editor's
corresponding diagnostic severity.

`status` is one of:

- `clean`: no violations;
- `violations`: `diagnostics` is nonempty;
- `precondition_failed`: linting was skipped and `precondition` describes why;
- `error`: the run could not complete and `error` contains a message.

The optional output file contains only the latest complete snapshot as formatted
JSON, not the `run_started` messages. It is atomically replaced after a
completed run.
