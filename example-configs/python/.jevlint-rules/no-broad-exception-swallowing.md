# No broad exception swallowing

Fail when production code catches `Exception`, `BaseException`, or uses a bare
`except` and then silently ignores the error or returns a misleading successful
result. Logging alone does not necessarily make recovery valid. Do not fail when
the exception is re-raised, deliberately translated to a more specific error,
or handled at a clearly documented process boundary.

