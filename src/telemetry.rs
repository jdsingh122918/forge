#[cfg(test)]
mod tests {
    #[test]
    fn test_tracing_imports_available() {
        // This will fail to compile if tracing isn't in Cargo.toml
        use tracing::info;
        info!("test");
    }
}
