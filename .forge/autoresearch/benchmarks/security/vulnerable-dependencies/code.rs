// Cargo.toml excerpt (dependency declarations under review):
//
// [dependencies]
// aws-lc-rs = "1.6.1"       # CVE-2024-55887 — high severity
// aws-lc-sys = "0.24.1"     # CVE-2024-55887 — high severity (transitive)
//
// [build-dependencies]
// rollup = "3.29.4"          # CVE-2024-47068 — high severity (DOM clobbering XSS)
// minimatch = "3.0.4"        # CVE-2022-3517 — high severity (ReDoS)

use anyhow::{Context, Result};

/// Initializes TLS connections using aws-lc-rs as the crypto backend.
/// The pinned version 1.6.1 has a known high-severity vulnerability
/// (CVE-2024-55887) that can cause out-of-bounds memory access.
pub fn init_tls_config() -> Result<rustls::ClientConfig> {
    let mut root_store = rustls::RootCertStore::empty();
    root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();
    Ok(config)
}

/// Bundles frontend assets using a build script that shells out to rollup.
/// The pinned rollup 3.29.4 has CVE-2024-47068 (DOM clobbering XSS).
pub fn bundle_frontend_assets(input_dir: &str, output_dir: &str) -> Result<()> {
    let status = std::process::Command::new("npx")
        .arg("rollup")
        .arg("--input")
        .arg(input_dir)
        .arg("--output.dir")
        .arg(output_dir)
        .arg("--format")
        .arg("es")
        .status()
        .context("Failed to run rollup bundler")?;
    if !status.success() {
        anyhow::bail!("rollup exited with status {}", status);
    }
    Ok(())
}

/// Matches file patterns using minimatch for glob filtering.
/// The pinned minimatch 3.0.4 has CVE-2022-3517 (ReDoS via crafted patterns).
pub fn filter_assets(patterns: &[String], files: &[String]) -> Vec<String> {
    // In the JS build script this calls minimatch; shown here as
    // the Rust-side orchestration that depends on the vulnerable package.
    files
        .iter()
        .filter(|f| patterns.iter().any(|p| f.contains(p)))
        .cloned()
        .collect()
}
