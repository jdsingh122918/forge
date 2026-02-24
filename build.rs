fn main() {
    // Only build frontend in release mode or when FORGE_BUILD_UI=1
    if std::env::var("PROFILE").unwrap_or_default() == "release"
        || std::env::var("FORGE_BUILD_UI").is_ok()
    {
        println!("cargo:rerun-if-changed=ui/src");
        println!("cargo:rerun-if-changed=ui/index.html");
        println!("cargo:rerun-if-changed=ui/package.json");

        let ui_dir = std::path::Path::new("ui");
        if !ui_dir.join("node_modules").exists() {
            let install = std::process::Command::new("npm")
                .args(["install"])
                .current_dir("ui")
                .status()
                .expect("Failed to run npm install");
            assert!(install.success(), "npm install failed");
        }

        let status = std::process::Command::new("npm")
            .args(["run", "build"])
            .current_dir("ui")
            .status()
            .expect("Failed to run npm build");

        assert!(status.success(), "npm build failed");
    }
}
