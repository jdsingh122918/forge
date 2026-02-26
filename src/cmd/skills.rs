//! Skills management commands — `forge skills`.

use anyhow::{Context, Result};

use super::super::SkillsCommands;

pub fn cmd_skills(project_dir: &std::path::Path, command: Option<SkillsCommands>) -> Result<()> {
    use dialoguer::Confirm;
    use forge::init::get_forge_dir;
    use forge::skills::{SkillsLoader, create_skill, delete_skill};

    let forge_dir = get_forge_dir(project_dir);
    let mut loader = SkillsLoader::new(&forge_dir, false);

    match command {
        None | Some(SkillsCommands::List) => {
            // List all skills
            println!();
            println!("Available Skills");
            println!("================");
            println!();

            if !loader.skills_dir_exists() {
                println!("No skills directory found.");
                println!();
                println!("Run 'forge init' to create the directory structure,");
                println!("or 'forge skills create <name>' to create your first skill.");
                println!();
                return Ok(());
            }

            let skills = loader.list_skills()?;
            if skills.is_empty() {
                println!("No skills found in {}", loader.skills_dir().display());
                println!();
                println!("Create a skill with:");
                println!("  forge skills create <name>");
                println!();
            } else {
                println!("Skills directory: {}", loader.skills_dir().display());
                println!();
                for skill_name in &skills {
                    println!("  - {}", skill_name);
                }
                println!();
                println!("{} skill(s) available", skills.len());
                println!();
                println!("Use 'forge skills show <name>' to view a skill's content.");
                println!();
            }
        }
        Some(SkillsCommands::Show { name }) => {
            // Show skill content
            match loader.load_skill(&name)? {
                Some(skill) => {
                    println!();
                    println!("Skill: {}", skill.name);
                    println!("Path:  {}", skill.path.display());
                    println!();
                    println!("--- Content ---");
                    println!("{}", skill.content);
                    println!("--- End ---");
                    println!();
                }
                None => {
                    println!("Skill '{}' not found.", name);
                    println!();
                    println!("Run 'forge skills' to see available skills.");
                }
            }
        }
        Some(SkillsCommands::Create { name, file }) => {
            // Create a new skill
            let content = if let Some(file_path) = file {
                std::fs::read_to_string(&file_path)
                    .with_context(|| format!("Failed to read file: {}", file_path.display()))?
            } else {
                // Read from stdin if no file provided
                println!("Enter skill content (Ctrl+D when done):");
                let mut content = String::new();
                use std::io::Read;
                std::io::stdin().read_to_string(&mut content)?;
                content
            };

            if content.trim().is_empty() {
                anyhow::bail!("Skill content cannot be empty");
            }

            let skill_dir = create_skill(&forge_dir, &name, &content)?;
            println!("Created skill '{}' at {}", name, skill_dir.display());
            println!();
            println!("Use this skill in phases by adding it to the 'skills' field:");
            println!("  {{");
            println!("    \"number\": \"01\",");
            println!("    \"name\": \"My Phase\",");
            println!("    \"skills\": [\"{}\"],", name);
            println!("    ...");
            println!("  }}");
            println!();
            println!("Or set it as a global skill in forge.toml:");
            println!("  [skills]");
            println!("  global = [\"{}\"]", name);
            println!();
        }
        Some(SkillsCommands::Delete { name, force }) => {
            // Delete a skill
            let skill_path = loader.skill_path(&name);
            if !skill_path.exists() {
                println!("Skill '{}' not found.", name);
                return Ok(());
            }

            if !force {
                let confirm = Confirm::new()
                    .with_prompt(format!("Delete skill '{}'?", name))
                    .default(false)
                    .interact()
                    .unwrap_or(false);

                if !confirm {
                    println!("Deletion cancelled.");
                    return Ok(());
                }
            }

            delete_skill(&forge_dir, &name)?;
            println!("Deleted skill '{}'", name);
        }
    }

    Ok(())
}
