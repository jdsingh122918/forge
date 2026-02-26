//! Configuration view and validation commands — `forge config`.

use anyhow::Result;

use super::super::ConfigCommands;

pub fn cmd_config(project_dir: &std::path::Path, command: Option<ConfigCommands>) -> Result<()> {
    use forge::forge_config::{ForgeConfig, ForgeToml};
    use forge::init::get_forge_dir;

    let forge_dir = get_forge_dir(project_dir);
    let config_path = forge_dir.join("forge.toml");

    match command {
        None | Some(ConfigCommands::Show) => {
            // Show current configuration
            println!();
            println!("Forge Configuration");
            println!("===================");
            println!();

            if config_path.exists() {
                println!("Config file: {}", config_path.display());
                println!();

                let toml = ForgeToml::load(&config_path)?;

                // Project section
                if toml.project.name.is_some() || toml.project.claude_cmd.is_some() {
                    println!("[project]");
                    if let Some(name) = &toml.project.name {
                        println!("  name = \"{}\"", name);
                    }
                    if let Some(cmd) = &toml.project.claude_cmd {
                        println!("  claude_cmd = \"{}\"", cmd);
                    }
                    println!();
                }

                // Defaults section
                println!("[defaults]");
                println!("  budget = {}", toml.defaults.budget);
                println!(
                    "  auto_approve_threshold = {}",
                    toml.defaults.auto_approve_threshold
                );
                println!("  permission_mode = \"{}\"", toml.defaults.permission_mode);
                println!("  context_limit = \"{}\"", toml.defaults.context_limit);
                println!("  skip_permissions = {}", toml.defaults.skip_permissions);
                println!();

                // Phase overrides
                if !toml.phases.overrides.is_empty() {
                    println!("[phases.overrides]");
                    for (pattern, override_cfg) in &toml.phases.overrides {
                        println!("  \"{}\":", pattern);
                        if let Some(budget) = override_cfg.budget {
                            println!("    budget = {}", budget);
                        }
                        if let Some(mode) = override_cfg.permission_mode {
                            println!("    permission_mode = \"{}\"", mode);
                        }
                        if let Some(limit) = &override_cfg.context_limit {
                            println!("    context_limit = \"{}\"", limit);
                        }
                    }
                    println!();
                }

                // Show effective values (including env overrides)
                println!("Effective values (with env/CLI overrides):");
                let config = ForgeConfig::new(project_dir.to_path_buf())?;
                println!("  claude_cmd = \"{}\"", config.claude_cmd());
                println!("  skip_permissions = {}", config.skip_permissions());
                println!();
            } else {
                println!("No forge.toml found at {}", config_path.display());
                println!();
                println!("Using default configuration:");
                let toml = ForgeToml::default();
                println!("  budget = {}", toml.defaults.budget);
                println!(
                    "  auto_approve_threshold = {}",
                    toml.defaults.auto_approve_threshold
                );
                println!("  permission_mode = \"{}\"", toml.defaults.permission_mode);
                println!("  context_limit = \"{}\"", toml.defaults.context_limit);
                println!("  skip_permissions = {}", toml.defaults.skip_permissions);
                println!();
                println!("Run 'forge config init' to create a forge.toml file.");
                println!();
            }
        }
        Some(ConfigCommands::Validate) => {
            // Validate configuration
            println!();
            println!("Validating configuration...");
            println!();

            if !config_path.exists() {
                println!("No forge.toml found. Using defaults (valid).");
                return Ok(());
            }

            let toml = ForgeToml::load(&config_path)?;
            let warnings = toml.validate();

            if warnings.is_empty() {
                println!("Configuration is valid.");
            } else {
                println!("Configuration warnings:");
                for warning in warnings {
                    println!("  - {}", warning);
                }
            }
            println!();
        }
        Some(ConfigCommands::Init) => {
            // Initialize default forge.toml
            if config_path.exists() {
                println!("forge.toml already exists at {}", config_path.display());
                println!("Delete it first if you want to recreate it.");
                return Ok(());
            }

            // Ensure .forge directory exists
            if !forge_dir.exists() {
                std::fs::create_dir_all(&forge_dir)?;
            }

            let toml = ForgeToml::default();
            toml.save(&config_path)?;

            println!("Created forge.toml at {}", config_path.display());
            println!();
            println!("You can now customize:");
            println!("  - [project] name, claude_cmd");
            println!("  - [defaults] budget, permission_mode, context_limit");
            println!("  - [phases.overrides.\"pattern-*\"] for phase-specific settings");
            println!();
        }
    }

    Ok(())
}
