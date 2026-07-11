# Python v3 compatibility oracle

This immutable corpus was captured from the live former CLI and registry before
their Phase 2 deletion. The historical capture command was:

```sh
python3 tools/capture_phase2_compat.py
```

The removed capture helper seeded current, legacy, exact-usage, killed, orphaned, RPC
`agent_end`, session-linked, retry, handoff, corrupt, truncated, CJK,
combining-mark, and wide-character records. It invokes Python and Rust `list
--json`, `show`, `stats --json`, and cached `quota --json`, records exit codes,
and replaces the temporary registry prefix with `<ORC_HOME>`.

Tests compare parsed JSON and exit structure, not timestamps, whitespace, or
temporary paths. Unknown top-level and token fields are invariants and must
survive every Rust read/update/write round trip.
