# No unvalidated dynamic data

Fail when untrusted dynamic data, such as decoded JSON, request input,
environment values, or deserialized messages, is used as a trusted application
object without validation. Do not fail when this file validates the relevant
shape and values, or when the data remains opaque and is only forwarded safely.

