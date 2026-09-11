# Browser arithmetic experiments

This branch adds benchmark instrumentation and an asynchronous arithmetic bridge.
The existing Arkworks branch remains the deployment baseline. Timing fields are
local diagnostics, are nested (not additive), and are not proof statements.

`browser.cjs` records full proof times, first-proof latency, preparation time,
worker pools and per-proof phase timings. Verification runs outside proof timing.
`cmd/reencode` converts existing keys with `WriteRawTo` and checks an exact
compressed round trip. It never creates another setup or skips validation.

JavaScript backend comparisons use the same Go binary, fixtures and API. Worker
requests are serialized even when a backend yields. Hybrid Rust helpers are
compiled only with the `hybrid` feature; `bench-profile` enables Rust timers.
The JavaScript libraries in this directory are benchmark dependencies, not
production package defaults. See each backend branch and the results report for
reproduction commands and limitations.
