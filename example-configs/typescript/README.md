# TypeScript example

This configuration lints TypeScript and TSX source files while excluding common
dependency, build, coverage, declaration, and generated outputs. It includes two
semantic rules that complement ordinary static analysis: floating promises and
unvalidated casts at trust boundaries.

The precondition runs once for the whole project and requires both commands to
succeed:

```sh
pnpm exec tsc --noEmit --pretty false
pnpm exec biome check .
```

Install TypeScript and Biome in the target project before using it. A failure in
either command skips Jev linting for every selected file.

Copy `.jevlintrc.toml`, `.jevlint-system.md`, and `.jevlint-rules/` to the root
of the target project, or invoke the example directly from this repository:

```sh
jevlint --config example-configs/typescript/.jevlintrc.toml --dry-run
```

Relative paths and the precondition working directory are based on the folder
containing the selected configuration file. For a real project, copying the
files to its root is therefore the usual setup.

