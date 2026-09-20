# Python example

This configuration lints Python source and stub files while excluding common
virtual environments, caches, build products, migrations, and generated protobuf
modules. Its semantic rules focus on swallowed exceptions and unchecked dynamic
data at trust boundaries.

The precondition runs once for the whole project and requires both commands to
succeed:

```sh
pyright
ruff check .
```

Install Pyright and Ruff in the environment used to run `jevlint`. A failure in
either command skips Jev linting for every selected file. If your project runs
tools through `uv`, replace the command with, for example:

```toml
[precondition]
command = ["sh", "-c", "uv run pyright && uv run ruff check ."]
```

Copy `.jevlintrc.toml`, `.jevlint-system.md`, and `.jevlint-rules/` to the root
of the target project. Relative paths and the precondition working directory are
based on the folder containing the configuration file.
