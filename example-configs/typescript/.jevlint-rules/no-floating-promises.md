# No floating promises

A Promise is started without being awaited, returned, handled, or documented.

---

Fail when an asynchronous operation returning a Promise is started without being
awaited, returned, explicitly handled with `.then`/`.catch`, or deliberately
discarded with `void`. Do not fail for synchronous calls or for calls whose
return type cannot reasonably be inferred to be a Promise from this file.
