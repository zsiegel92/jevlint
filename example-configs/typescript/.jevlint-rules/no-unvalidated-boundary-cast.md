# No unvalidated boundary casts

Fail when data from an untrusted boundary, such as an HTTP response, parsed JSON,
environment input, storage, or a message payload, is asserted directly to an
application type without runtime validation. Do not fail for casts involving
values already validated in this file or for ordinary internal type narrowing.

