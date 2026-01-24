//! Skills and Templates System for Forge.
//!
//! This module provides reusable prompt fragments loaded on-demand that reduce
//! repetition and provide specialized context for different phase types.
//!
//! # Skill Structure
//!
//! Skills are stored as markdown files in `.forge/skills/`:
//!
//! ```text
//! .forge/skills/
//! ├── rust-conventions/
//! │   └── SKILL.md      # Rust-specific guidance
//! ├── testing-strategy/
//! │   └── SKILL.md      # Testing approach
//! └── api-design/
//!     └── SKILL.md      # REST/GraphQL patterns
//! ```
//!
//! # Phase Integration
//!
//! Phases can reference skills in their JSON definition:
//!
//! ```json
//! {
//!   "number": "03",
//!   "name": "API implementation",
//!   "skills": ["rust-conventions", "api-design"],
//!   "promise": "API COMPLETE"
//! }
//! ```
//!
//! Skills are injected into the phase prompt between the specification and task sections.

use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// The name of the skills directory within .forge
pub const SKILLS_DIR: &str = "skills";

/// The filename for skill content
pub const SKILL_FILE: &str = "SKILL.md";

/// A loaded skill with its content and metadata.
#[derive(Debug, Clone)]
pub struct Skill {
    /// The skill name (directory name)
    pub name: String,
    /// The full path to the skill directory
    pub path: PathBuf,
    /// The content of SKILL.md
    pub content: String,
}

impl Skill {
    /// Create a new Skill from a path and content.
    pub fn new(name: &str, path: PathBuf, content: String) -> Self {
        Self {
            name: name.to_string(),
            path,
            content,
        }
    }

    /// Get the skill content formatted for injection into a prompt.
    pub fn as_prompt_section(&self) -> String {
        format!(
            "## SKILL: {}\n\n{}",
            self.name.to_uppercase().replace('-', " "),
            self.content.trim()
        )
    }
}

/// Skills loader that manages loading and caching of skills.
#[derive(Debug)]
pub struct SkillsLoader {
    /// Path to the skills directory (.forge/skills)
    skills_dir: PathBuf,
    /// Cache of loaded skills
    cache: HashMap<String, Skill>,
    /// Whether verbose logging is enabled
    verbose: bool,
}

impl SkillsLoader {
    /// Create a new SkillsLoader for the given forge directory.
    pub fn new(forge_dir: &Path, verbose: bool) -> Self {
        Self {
            skills_dir: forge_dir.join(SKILLS_DIR),
            cache: HashMap::new(),
            verbose,
        }
    }

    /// Load a single skill by name.
    ///
    /// Returns None if the skill doesn't exist (with a warning if verbose).
    /// Returns an error only for I/O failures on existing skills.
    pub fn load_skill(&mut self, name: &str) -> Result<Option<Skill>> {
        // Check cache first
        if let Some(skill) = self.cache.get(name) {
            return Ok(Some(skill.clone()));
        }

        let skill_dir = self.skills_dir.join(name);
        let skill_file = skill_dir.join(SKILL_FILE);

        if !skill_file.exists() {
            if self.verbose {
                eprintln!("Warning: Skill '{}' not found at {}", name, skill_file.display());
            }
            return Ok(None);
        }

        let content = std::fs::read_to_string(&skill_file)
            .with_context(|| format!("Failed to read skill file: {}", skill_file.display()))?;

        let skill = Skill::new(name, skill_dir, content);
        self.cache.insert(name.to_string(), skill.clone());

        Ok(Some(skill))
    }

    /// Load multiple skills by name.
    ///
    /// Returns all successfully loaded skills, skipping missing ones.
    pub fn load_skills(&mut self, names: &[String]) -> Result<Vec<Skill>> {
        let mut skills = Vec::new();
        for name in names {
            if let Some(skill) = self.load_skill(name)? {
                skills.push(skill);
            }
        }
        Ok(skills)
    }

    /// Generate a prompt section from a list of skill names.
    ///
    /// This is the main entry point for integrating skills into prompts.
    /// Returns an empty string if no skills are found.
    pub fn generate_skills_section(&mut self, skill_names: &[String]) -> Result<String> {
        let skills = self.load_skills(skill_names)?;

        if skills.is_empty() {
            return Ok(String::new());
        }

        let sections: Vec<String> = skills.iter().map(|s| s.as_prompt_section()).collect();

        Ok(format!(
            "## SKILLS AND CONVENTIONS\n\nThe following skills provide guidance for this phase:\n\n{}\n\n",
            sections.join("\n\n---\n\n")
        ))
    }

    /// List all available skills in the skills directory.
    pub fn list_skills(&self) -> Result<Vec<String>> {
        if !self.skills_dir.exists() {
            return Ok(Vec::new());
        }

        let mut skills = Vec::new();
        let entries = std::fs::read_dir(&self.skills_dir)
            .with_context(|| format!("Failed to read skills directory: {}", self.skills_dir.display()))?;

        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir()
                && path.join(SKILL_FILE).exists()
                && let Some(name) = path.file_name().and_then(|n| n.to_str())
            {
                skills.push(name.to_string());
            }
        }

        skills.sort();
        Ok(skills)
    }

    /// Get the path where a new skill should be created.
    pub fn skill_path(&self, name: &str) -> PathBuf {
        self.skills_dir.join(name)
    }

    /// Check if the skills directory exists.
    pub fn skills_dir_exists(&self) -> bool {
        self.skills_dir.exists()
    }

    /// Get the skills directory path.
    pub fn skills_dir(&self) -> &Path {
        &self.skills_dir
    }
}

/// Create the skills directory structure.
pub fn ensure_skills_directory(forge_dir: &Path) -> Result<PathBuf> {
    let skills_dir = forge_dir.join(SKILLS_DIR);
    std::fs::create_dir_all(&skills_dir)
        .with_context(|| format!("Failed to create skills directory: {}", skills_dir.display()))?;
    Ok(skills_dir)
}

/// Create a new skill with the given name and content.
pub fn create_skill(forge_dir: &Path, name: &str, content: &str) -> Result<PathBuf> {
    let skills_dir = ensure_skills_directory(forge_dir)?;
    let skill_dir = skills_dir.join(name);
    std::fs::create_dir_all(&skill_dir)
        .with_context(|| format!("Failed to create skill directory: {}", skill_dir.display()))?;

    let skill_file = skill_dir.join(SKILL_FILE);
    std::fs::write(&skill_file, content)
        .with_context(|| format!("Failed to write skill file: {}", skill_file.display()))?;

    Ok(skill_dir)
}

/// Delete a skill by name.
pub fn delete_skill(forge_dir: &Path, name: &str) -> Result<()> {
    let skill_dir = forge_dir.join(SKILLS_DIR).join(name);
    if skill_dir.exists() {
        std::fs::remove_dir_all(&skill_dir)
            .with_context(|| format!("Failed to delete skill: {}", skill_dir.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn create_test_skill(forge_dir: &Path, name: &str, content: &str) -> PathBuf {
        let skill_dir = forge_dir.join(SKILLS_DIR).join(name);
        std::fs::create_dir_all(&skill_dir).unwrap();
        let skill_file = skill_dir.join(SKILL_FILE);
        std::fs::write(&skill_file, content).unwrap();
        skill_dir
    }

    // =========================================
    // Skill struct tests
    // =========================================

    #[test]
    fn test_skill_new() {
        let skill = Skill::new("rust-conventions", PathBuf::from("/test"), "Use clippy".to_string());
        assert_eq!(skill.name, "rust-conventions");
        assert_eq!(skill.content, "Use clippy");
    }

    #[test]
    fn test_skill_as_prompt_section() {
        let skill = Skill::new("rust-conventions", PathBuf::from("/test"), "Use clippy.\nRun tests.".to_string());
        let section = skill.as_prompt_section();
        assert!(section.contains("## SKILL: RUST CONVENTIONS"));
        assert!(section.contains("Use clippy."));
        assert!(section.contains("Run tests."));
    }

    // =========================================
    // SkillsLoader tests
    // =========================================

    #[test]
    fn test_skills_loader_new() {
        let dir = tempdir().unwrap();
        let forge_dir = dir.path().join(".forge");
        std::fs::create_dir_all(&forge_dir).unwrap();

        let loader = SkillsLoader::new(&forge_dir, false);
        assert_eq!(loader.skills_dir, forge_dir.join("skills"));
        assert!(loader.cache.is_empty());
    }

    #[test]
    fn test_skills_loader_load_skill_success() {
        let dir = tempdir().unwrap();
        let forge_dir = dir.path().join(".forge");
        create_test_skill(&forge_dir, "my-skill", "# My Skill\n\nDo the thing.");

        let mut loader = SkillsLoader::new(&forge_dir, false);
        let skill = loader.load_skill("my-skill").unwrap().unwrap();

        assert_eq!(skill.name, "my-skill");
        assert!(skill.content.contains("# My Skill"));
        assert!(skill.content.contains("Do the thing."));
    }

    #[test]
    fn test_skills_loader_load_skill_not_found() {
        let dir = tempdir().unwrap();
        let forge_dir = dir.path().join(".forge");
        std::fs::create_dir_all(forge_dir.join("skills")).unwrap();

        let mut loader = SkillsLoader::new(&forge_dir, false);
        let result = loader.load_skill("nonexistent").unwrap();

        assert!(result.is_none());
    }

    #[test]
    fn test_skills_loader_caching() {
        let dir = tempdir().unwrap();
        let forge_dir = dir.path().join(".forge");
        create_test_skill(&forge_dir, "cached-skill", "Original content");

        let mut loader = SkillsLoader::new(&forge_dir, false);

        // First load
        let skill1 = loader.load_skill("cached-skill").unwrap().unwrap();
        assert!(skill1.content.contains("Original content"));

        // Modify file (should not affect cached result)
        let skill_file = forge_dir.join("skills/cached-skill/SKILL.md");
        std::fs::write(&skill_file, "Modified content").unwrap();

        // Second load should return cached version
        let skill2 = loader.load_skill("cached-skill").unwrap().unwrap();
        assert!(skill2.content.contains("Original content"));
    }

    #[test]
    fn test_skills_loader_load_multiple() {
        let dir = tempdir().unwrap();
        let forge_dir = dir.path().join(".forge");
        create_test_skill(&forge_dir, "skill-a", "Content A");
        create_test_skill(&forge_dir, "skill-b", "Content B");

        let mut loader = SkillsLoader::new(&forge_dir, false);
        let names = vec!["skill-a".to_string(), "skill-b".to_string(), "nonexistent".to_string()];
        let skills = loader.load_skills(&names).unwrap();

        assert_eq!(skills.len(), 2);
        assert_eq!(skills[0].name, "skill-a");
        assert_eq!(skills[1].name, "skill-b");
    }

    #[test]
    fn test_skills_loader_generate_section() {
        let dir = tempdir().unwrap();
        let forge_dir = dir.path().join(".forge");
        create_test_skill(&forge_dir, "rust-conventions", "Use Rust idioms.");
        create_test_skill(&forge_dir, "testing-strategy", "Write unit tests.");

        let mut loader = SkillsLoader::new(&forge_dir, false);
        let names = vec!["rust-conventions".to_string(), "testing-strategy".to_string()];
        let section = loader.generate_skills_section(&names).unwrap();

        assert!(section.contains("## SKILLS AND CONVENTIONS"));
        assert!(section.contains("## SKILL: RUST CONVENTIONS"));
        assert!(section.contains("Use Rust idioms."));
        assert!(section.contains("## SKILL: TESTING STRATEGY"));
        assert!(section.contains("Write unit tests."));
        assert!(section.contains("---")); // Separator between skills
    }

    #[test]
    fn test_skills_loader_generate_section_empty() {
        let dir = tempdir().unwrap();
        let forge_dir = dir.path().join(".forge");
        std::fs::create_dir_all(forge_dir.join("skills")).unwrap();

        let mut loader = SkillsLoader::new(&forge_dir, false);
        let names = vec!["nonexistent".to_string()];
        let section = loader.generate_skills_section(&names).unwrap();

        assert!(section.is_empty());
    }

    #[test]
    fn test_skills_loader_list_skills() {
        let dir = tempdir().unwrap();
        let forge_dir = dir.path().join(".forge");
        create_test_skill(&forge_dir, "skill-z", "Z");
        create_test_skill(&forge_dir, "skill-a", "A");
        create_test_skill(&forge_dir, "skill-m", "M");

        let loader = SkillsLoader::new(&forge_dir, false);
        let skills = loader.list_skills().unwrap();

        assert_eq!(skills, vec!["skill-a", "skill-m", "skill-z"]);
    }

    #[test]
    fn test_skills_loader_list_skills_empty() {
        let dir = tempdir().unwrap();
        let forge_dir = dir.path().join(".forge");
        std::fs::create_dir_all(forge_dir.join("skills")).unwrap();

        let loader = SkillsLoader::new(&forge_dir, false);
        let skills = loader.list_skills().unwrap();

        assert!(skills.is_empty());
    }

    #[test]
    fn test_skills_loader_list_skills_ignores_invalid() {
        let dir = tempdir().unwrap();
        let forge_dir = dir.path().join(".forge");
        create_test_skill(&forge_dir, "valid-skill", "Valid");

        // Create a directory without SKILL.md (should be ignored)
        std::fs::create_dir_all(forge_dir.join("skills/incomplete-skill")).unwrap();
        std::fs::write(forge_dir.join("skills/incomplete-skill/README.md"), "Not a skill").unwrap();

        let loader = SkillsLoader::new(&forge_dir, false);
        let skills = loader.list_skills().unwrap();

        assert_eq!(skills, vec!["valid-skill"]);
    }

    // =========================================
    // Helper function tests
    // =========================================

    #[test]
    fn test_ensure_skills_directory() {
        let dir = tempdir().unwrap();
        let forge_dir = dir.path().join(".forge");
        std::fs::create_dir_all(&forge_dir).unwrap();

        let skills_dir = ensure_skills_directory(&forge_dir).unwrap();
        assert!(skills_dir.exists());
        assert_eq!(skills_dir, forge_dir.join("skills"));
    }

    #[test]
    fn test_create_skill() {
        let dir = tempdir().unwrap();
        let forge_dir = dir.path().join(".forge");
        std::fs::create_dir_all(&forge_dir).unwrap();

        let skill_dir = create_skill(&forge_dir, "new-skill", "# New Skill\n\nContent here.").unwrap();

        assert!(skill_dir.exists());
        let content = std::fs::read_to_string(skill_dir.join("SKILL.md")).unwrap();
        assert!(content.contains("# New Skill"));
    }

    #[test]
    fn test_delete_skill() {
        let dir = tempdir().unwrap();
        let forge_dir = dir.path().join(".forge");
        create_test_skill(&forge_dir, "to-delete", "Content");

        let skill_dir = forge_dir.join("skills/to-delete");
        assert!(skill_dir.exists());

        delete_skill(&forge_dir, "to-delete").unwrap();
        assert!(!skill_dir.exists());
    }

    #[test]
    fn test_delete_skill_nonexistent() {
        let dir = tempdir().unwrap();
        let forge_dir = dir.path().join(".forge");
        std::fs::create_dir_all(forge_dir.join("skills")).unwrap();

        // Should not error on nonexistent skill
        let result = delete_skill(&forge_dir, "nonexistent");
        assert!(result.is_ok());
    }
}
