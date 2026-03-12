# Circular Dependencies: executor <-> phase_manager <-> reporter

This is a synthetic benchmark case demonstrating circular import dependencies
between three modules in a pipeline orchestration system:

- **executor** depends on phase_manager and reporter
- **phase_manager** depends on executor and reporter
- **reporter** depends on executor and phase_manager

The circular dependencies create several problems:

1. **Ownership cycles** — Rust's ownership model cannot express mutual ownership,
   forcing the use of raw pointers (`*mut`, `*const`) to break the cycle
2. **Unsafe code** — Raw pointer dereferencing requires `unsafe` blocks, bypassing
   Rust's safety guarantees
3. **Multi-phase initialization** — Objects cannot be fully constructed in one step;
   they require post-construction `set_*` calls to wire back-references
4. **Untestable in isolation** — No module can be unit-tested without constructing
   the entire dependency graph
5. **Memory safety risk** — Raw pointers can become dangling if any struct is moved
   or dropped while others still hold references

The fix is to introduce event-based communication (channels) or break the cycle by
extracting shared state into a separate module that all three depend on.
