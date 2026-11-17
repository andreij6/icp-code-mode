# candid 0.10.40, patched for icp-code-mode

Copied from crates.io unchanged except for the lines marked `icp-code-mode patch`:

- `src/utils.rs` `check_recursion_depth`: a fixed depth bound of 1024 on every target.
  Upstream's guard is a no-op on wasm32, so a deeply nested reply overflowed the stack.
- `src/de.rs` `recoverable_visit_some`: the snapshot copies the subtype memo, so its size is
  charged to the decoding quota (a hostile reply could otherwise make each `opt` cost the
  whole memo for free).
