# Vulnerable Dependencies

This code is from a build and networking module that depends on several packages
with known high-severity CVEs. The `code.rs` file shows how the vulnerable
dependencies are used; the security issue is in the dependency versions, not in
the Rust logic itself.

Vulnerable dependencies (pinned in Cargo.toml / package.json):
- **aws-lc-rs 1.6.1 / aws-lc-sys 0.24.1** — CVE-2024-55887: out-of-bounds memory access in the AWS-LC cryptographic library.
- **rollup 3.29.4** — CVE-2024-47068: DOM clobbering XSS through crafted module names in bundled output.
- **minimatch 3.0.4** — CVE-2022-3517: Regular Expression Denial of Service (ReDoS) via specially crafted glob patterns.

The fix is to upgrade each dependency to a patched version.
