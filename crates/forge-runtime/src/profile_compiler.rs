//! Profile compiler for trusted runtime profiles and untrusted project overlays.
//!
//! Project overlays are TOML data, never executable code. The compiler validates
//! overlays against a trusted base schema, merges the allowed reductions and
//! optional capability enablements, and produces `forge_common::manifest::CompiledProfile`.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::ffi::OsStr;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use dashmap::DashMap;
use forge_common::manifest::{
    AgentManifest, CompiledProfile, CredentialAccessMode, CredentialGrant, MemoryPolicy,
    MemoryScope, PermissionSet, RepoAccess, ResourceLimits, RunSharedWriteMode, RuntimeEnvPlan,
    SpawnLimits,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::Semaphore;
use tracing::debug;

/// Cache key for compiled profile lookup.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ProfileKey {
    base_profile: String,
    overlay_hash: Option<String>,
}

/// Trusted base profile schema owned by Forge.
///
/// The base profile carries exact runtime and manifest defaults plus the
/// optional capabilities a project overlay is allowed to request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustedBaseProfile {
    /// Stable trusted profile name, for example `implementer`.
    pub name: String,
    /// Runtime environment plan selected by trusted config.
    pub env_plan: RuntimeEnvPlan,
    /// Tools always present in the compiled manifest.
    pub tools: Vec<String>,
    /// Optional tool identifiers a project overlay may enable.
    ///
    /// The key is the overlay-facing identifier. The value is the manifest tool
    /// entry to append when that identifier is enabled.
    pub optional_tools: BTreeMap<String, String>,
    /// MCP servers always granted by this profile.
    pub mcp_servers: Vec<String>,
    /// Credentials always granted by this profile.
    pub credentials: Vec<CredentialGrant>,
    /// Optional credential handles a project overlay may enable.
    ///
    /// The key is the overlay-facing identifier. The value is the full grant
    /// copied into the compiled manifest.
    pub optional_credentials: BTreeMap<String, CredentialGrant>,
    /// Baseline memory policy.
    pub memory_policy: MemoryPolicy,
    /// Baseline resource limits.
    pub resources: ResourceLimits,
    /// Baseline permissions.
    pub permissions: PermissionSet,
    /// Optional network destinations a project overlay may enable.
    pub optional_network_allowlist: BTreeSet<String>,
}

/// Untrusted project overlay parsed from TOML.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProjectOverlay {
    /// Trusted base profile this overlay extends.
    pub extends: String,
    /// Optional tool enablements.
    #[serde(default)]
    pub tools: Option<ToolsOverlay>,
    /// Optional credential enablements.
    #[serde(default)]
    pub credentials: Option<CredentialsOverlay>,
    /// Optional network host enablements.
    #[serde(default)]
    pub network: Option<NetworkOverlay>,
    /// Optional write-scope reductions.
    #[serde(default)]
    pub memory: Option<MemoryOverlay>,
    /// Optional spawn-limit reductions.
    #[serde(default)]
    pub spawn: Option<SpawnOverlay>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ToolsOverlay {
    #[serde(default)]
    pub enable: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CredentialsOverlay {
    #[serde(default)]
    pub enable: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct NetworkOverlay {
    #[serde(default)]
    pub enable: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MemoryOverlay {
    #[serde(default)]
    pub write: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SpawnOverlay {
    #[serde(default)]
    pub max_children: Option<u32>,
    #[serde(default)]
    pub require_approval_after: Option<u32>,
}

/// Compiler for trusted base profiles plus untrusted project overlays.
#[derive(Debug)]
pub struct ProfileCompiler {
    base_profiles: BTreeMap<String, TrustedBaseProfile>,
    compiled_cache: DashMap<ProfileKey, CompiledProfile>,
    compile_lock: Semaphore,
}

/// Build the default trusted base profiles shipped with the runtime daemon.
pub fn default_trusted_base_profiles() -> Vec<TrustedBaseProfile> {
    vec![default_implementer_base_profile()]
}

/// Build the default profile compiler used by daemon bootstrap.
pub fn default_profile_compiler() -> Result<ProfileCompiler> {
    ProfileCompiler::new(default_trusted_base_profiles())
}

impl ProfileCompiler {
    /// Build a compiler from trusted base profiles registered by the daemon.
    pub fn new(base_profiles: Vec<TrustedBaseProfile>) -> Result<Self> {
        let mut profiles = BTreeMap::new();
        for profile in base_profiles {
            let profile_name = profile.name.clone();
            if profiles.insert(profile_name.clone(), profile).is_some() {
                bail!("duplicate trusted base profile '{}'", profile_name);
            }
        }

        Ok(Self {
            base_profiles: profiles,
            compiled_cache: DashMap::new(),
            compile_lock: Semaphore::new(4),
        })
    }

    /// Compile a trusted base profile with an optional validated overlay.
    pub async fn compile(
        &self,
        base_name: &str,
        overlay: Option<&ProjectOverlay>,
    ) -> Result<CompiledProfile> {
        let _permit = self
            .compile_lock
            .acquire()
            .await
            .context("profile compilation semaphore closed")?;

        self.compile_sync(base_name, overlay)
    }

    /// Compile synchronously. Useful in tests and non-async setup code.
    pub fn compile_sync(
        &self,
        base_name: &str,
        overlay: Option<&ProjectOverlay>,
    ) -> Result<CompiledProfile> {
        let overlay_hash = overlay
            .map(hash_overlay)
            .transpose()
            .context("failed to hash project overlay")?;

        let cache_key = ProfileKey {
            base_profile: base_name.to_string(),
            overlay_hash: overlay_hash.clone(),
        };

        if let Some(cached) = self.compiled_cache.get(&cache_key) {
            debug!(base_profile = base_name, "profile compiler cache hit");
            return Ok(cached.clone());
        }

        let base = self.base_profiles.get(base_name).with_context(|| {
            format!(
                "unknown base profile '{base_name}'. Available profiles: {}",
                self.base_profiles
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })?;

        if let Some(overlay) = overlay {
            self.validate_overlay(base, overlay)?;
        }

        let compiled = self
            .merge_profile(base, overlay, overlay_hash)
            .with_context(|| format!("failed to compile base profile '{base_name}'"))?;

        self.compiled_cache.insert(cache_key, compiled.clone());
        Ok(compiled)
    }

    /// Parse an overlay from TOML content.
    pub fn parse_overlay_str(content: &str) -> Result<ProjectOverlay> {
        toml::from_str(content).context("failed to parse profile overlay TOML")
    }

    /// Parse an overlay from a TOML file on disk.
    pub fn parse_overlay(path: &Path) -> Result<ProjectOverlay> {
        let content = fs::read_to_string(path)
            .with_context(|| format!("failed to read overlay file: {}", path.display()))?;

        Self::parse_overlay_str(&content)
            .with_context(|| format!("failed to parse overlay file: {}", path.display()))
    }

    /// Parse every `*.toml` overlay file in a directory.
    pub fn parse_overlays_from_dir(dir: &Path) -> Result<Vec<ProjectOverlay>> {
        if !dir.exists() {
            return Ok(Vec::new());
        }

        let mut paths = fs::read_dir(dir)
            .with_context(|| format!("failed to read overlay directory: {}", dir.display()))?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<std::result::Result<Vec<_>, _>>()
            .with_context(|| format!("failed to enumerate overlay directory: {}", dir.display()))?;

        paths.sort();

        paths
            .into_iter()
            .filter(|path| path.extension() == Some(OsStr::new("toml")))
            .map(|path| Self::parse_overlay(&path))
            .collect()
    }

    #[cfg(test)]
    /// Clear cached compiled profiles.
    fn invalidate_cache(&self) {
        self.compiled_cache.clear();
    }

    #[cfg(test)]
    /// Number of cached compiled profiles.
    fn cache_size(&self) -> usize {
        self.compiled_cache.len()
    }

    fn validate_overlay(&self, base: &TrustedBaseProfile, overlay: &ProjectOverlay) -> Result<()> {
        if overlay.extends != base.name {
            bail!(
                "overlay extends '{}' but base profile is '{}'",
                overlay.extends,
                base.name
            );
        }

        if let Some(tools) = &overlay.tools {
            for requested in &tools.enable {
                if !base.optional_tools.contains_key(requested) {
                    bail!(
                        "overlay requests tool '{}' which is not optional in base profile '{}'. Optional tools: {}",
                        requested,
                        base.name,
                        base.optional_tools
                            .keys()
                            .cloned()
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                }
            }
        }

        if let Some(credentials) = &overlay.credentials {
            for requested in &credentials.enable {
                if !base.optional_credentials.contains_key(requested) {
                    bail!(
                        "overlay requests credential '{}' which is not optional in base profile '{}'. Optional credentials: {}",
                        requested,
                        base.name,
                        base.optional_credentials
                            .keys()
                            .cloned()
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                }
            }
        }

        if let Some(network) = &overlay.network {
            for requested in &network.enable {
                if !base.optional_network_allowlist.contains(requested) {
                    bail!(
                        "overlay requests network host '{}' which is not optional in base profile '{}'. Optional hosts: {}",
                        requested,
                        base.name,
                        base.optional_network_allowlist
                            .iter()
                            .cloned()
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                }
            }
        }

        if let Some(memory) = &overlay.memory {
            for requested in &memory.write {
                let scope = parse_memory_scope_name(requested)
                    .with_context(|| format!("invalid overlay memory write scope '{requested}'"))?;
                if !base.memory_policy.write_scopes.contains(&scope) {
                    bail!(
                        "overlay requests write scope '{}' which is not allowed by base profile '{}'",
                        requested,
                        base.name
                    );
                }
            }
        }

        if let Some(spawn) = &overlay.spawn {
            if let Some(max_children) = spawn.max_children
                && max_children > base.permissions.spawn_limits.max_children
            {
                bail!(
                    "overlay requests max_children={} which exceeds base profile '{}' limit of {}",
                    max_children,
                    base.name,
                    base.permissions.spawn_limits.max_children
                );
            }

            if let Some(require_approval_after) = spawn.require_approval_after
                && require_approval_after > base.permissions.spawn_limits.require_approval_after
            {
                bail!(
                    "overlay requests require_approval_after={} which exceeds base profile '{}' limit of {}",
                    require_approval_after,
                    base.name,
                    base.permissions.spawn_limits.require_approval_after
                );
            }

            if let Some(require_approval_after) = spawn.require_approval_after {
                let effective_max_children = spawn
                    .max_children
                    .unwrap_or(base.permissions.spawn_limits.max_children);
                if require_approval_after > effective_max_children {
                    bail!(
                        "overlay requests require_approval_after={} which exceeds effective max_children={}",
                        require_approval_after,
                        effective_max_children
                    );
                }
            }
        }

        Ok(())
    }

    fn merge_profile(
        &self,
        base: &TrustedBaseProfile,
        overlay: Option<&ProjectOverlay>,
        overlay_hash: Option<String>,
    ) -> Result<CompiledProfile> {
        let mut tools = base.tools.clone();
        if let Some(tool_overlay) = overlay.and_then(|overlay| overlay.tools.as_ref()) {
            for requested in &tool_overlay.enable {
                let tool = base
                    .optional_tools
                    .get(requested)
                    .with_context(|| format!("missing optional tool mapping for '{requested}'"))?;
                push_unique_string(&mut tools, tool.clone());
            }
        }

        let mut credentials = base.credentials.clone();
        if let Some(credential_overlay) = overlay.and_then(|overlay| overlay.credentials.as_ref()) {
            for requested in &credential_overlay.enable {
                let grant = base.optional_credentials.get(requested).with_context(|| {
                    format!("missing optional credential mapping for '{requested}'")
                })?;
                push_unique_credential(&mut credentials, grant.clone());
            }
        }

        let mut network_allowlist: HashSet<String> = base.permissions.network_allowlist.clone();
        if let Some(network_overlay) = overlay.and_then(|overlay| overlay.network.as_ref()) {
            for requested in &network_overlay.enable {
                network_allowlist.insert(requested.clone());
            }
        }

        let write_scopes = match overlay.and_then(|overlay| overlay.memory.as_ref()) {
            Some(memory_overlay) => memory_overlay
                .write
                .iter()
                .map(|scope| {
                    parse_memory_scope_name(scope)
                        .with_context(|| format!("invalid overlay memory write scope '{scope}'"))
                })
                .collect::<Result<Vec<_>>>()?,
            None => base.memory_policy.write_scopes.clone(),
        };

        let max_children = overlay
            .and_then(|overlay| overlay.spawn.as_ref())
            .and_then(|spawn| spawn.max_children)
            .unwrap_or(base.permissions.spawn_limits.max_children);
        let require_approval_after = match overlay
            .and_then(|overlay| overlay.spawn.as_ref())
            .and_then(|spawn| spawn.require_approval_after)
        {
            Some(require_approval_after) => require_approval_after,
            None => base
                .permissions
                .spawn_limits
                .require_approval_after
                .min(max_children),
        };

        let manifest = AgentManifest {
            name: base.name.clone(),
            tools,
            mcp_servers: base.mcp_servers.clone(),
            credentials,
            memory_policy: MemoryPolicy {
                read_scopes: base.memory_policy.read_scopes.clone(),
                write_scopes,
                run_shared_write_mode: base.memory_policy.run_shared_write_mode,
            },
            resources: base.resources.clone(),
            permissions: PermissionSet {
                repo_access: base.permissions.repo_access,
                network_allowlist,
                spawn_limits: SpawnLimits {
                    max_children,
                    require_approval_after,
                },
                allow_project_memory_promotion: base.permissions.allow_project_memory_promotion,
            },
        };

        Ok(CompiledProfile {
            base_profile: base.name.clone(),
            overlay_hash,
            manifest,
            env_plan: base.env_plan.clone(),
        })
    }
}

fn hash_overlay(overlay: &ProjectOverlay) -> Result<String> {
    let serialized = toml::to_string(overlay).context("failed to serialize overlay for hashing")?;
    let mut hasher = Sha256::new();
    hasher.update(serialized.as_bytes());
    Ok(hex::encode(hasher.finalize()))
}

fn parse_memory_scope_name(scope: &str) -> Result<MemoryScope> {
    match scope {
        "scratch" => Ok(MemoryScope::Scratch),
        "run" | "run-shared" | "run_shared" => Ok(MemoryScope::RunShared),
        "project" => Ok(MemoryScope::Project),
        other => bail!("unknown memory scope '{other}'. Valid values: scratch, run, project"),
    }
}

fn push_unique_string(values: &mut Vec<String>, value: String) {
    if !values.iter().any(|existing| existing == &value) {
        values.push(value);
    }
}

fn push_unique_credential(values: &mut Vec<CredentialGrant>, value: CredentialGrant) {
    if !values
        .iter()
        .any(|existing| existing.handle == value.handle)
    {
        values.push(value);
    }
}

fn default_implementer_base_profile() -> TrustedBaseProfile {
    TrustedBaseProfile {
        name: "implementer".into(),
        env_plan: RuntimeEnvPlan::Host {
            explicit_opt_in: true,
        },
        tools: vec!["git".into(), "cargo".into()],
        optional_tools: BTreeMap::from([
            ("python3".into(), "python3".into()),
            ("poetry".into(), "poetry".into()),
        ]),
        mcp_servers: vec!["filesystem".into(), "github".into()],
        credentials: vec![CredentialGrant {
            handle: "github-api".into(),
            access_mode: CredentialAccessMode::ProxyOnly,
        }],
        optional_credentials: BTreeMap::from([(
            "pypi-publish".into(),
            CredentialGrant {
                handle: "pypi-publish".into(),
                access_mode: CredentialAccessMode::ProxyOnly,
            },
        )]),
        memory_policy: MemoryPolicy {
            read_scopes: vec![
                MemoryScope::Scratch,
                MemoryScope::RunShared,
                MemoryScope::Project,
            ],
            write_scopes: vec![MemoryScope::Scratch, MemoryScope::RunShared],
            run_shared_write_mode: RunSharedWriteMode::AppendOnlyLane,
        },
        resources: ResourceLimits {
            cpu: 4.0,
            memory_bytes: 8 * 1024 * 1024 * 1024,
            token_budget: 200_000,
        },
        permissions: PermissionSet {
            repo_access: RepoAccess::ReadWrite,
            network_allowlist: HashSet::from([
                "crates.io".to_string(),
                "registry.npmjs.org".to_string(),
            ]),
            spawn_limits: SpawnLimits {
                max_children: 10,
                require_approval_after: 5,
            },
            allow_project_memory_promotion: false,
        },
        optional_network_allowlist: BTreeSet::from([
            "files.pythonhosted.org".to_string(),
            "pypi.org".to_string(),
        ]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_implementer_base() -> TrustedBaseProfile {
        default_implementer_base_profile()
    }

    #[test]
    fn compile_without_overlay_produces_base_manifest() {
        let compiler = ProfileCompiler::new(vec![make_implementer_base()]).unwrap();

        let compiled = compiler.compile_sync("implementer", None).unwrap();

        assert_eq!(compiled.base_profile, "implementer");
        assert!(compiled.overlay_hash.is_none());
        assert_eq!(compiled.manifest.name, "implementer");
        assert_eq!(compiled.manifest.tools, vec!["git", "cargo"]);
        assert_eq!(compiled.manifest.credentials.len(), 1);
        assert_eq!(compiled.manifest.credentials[0].handle, "github-api");
        assert!(matches!(
            compiled.env_plan,
            RuntimeEnvPlan::Host {
                explicit_opt_in: true
            }
        ));
    }

    #[test]
    fn compile_with_valid_overlay_merges_optional_capabilities() {
        let compiler = ProfileCompiler::new(vec![make_implementer_base()]).unwrap();
        let overlay = ProjectOverlay {
            extends: "implementer".into(),
            tools: Some(ToolsOverlay {
                enable: vec!["python3".into()],
            }),
            credentials: Some(CredentialsOverlay {
                enable: vec!["pypi-publish".into()],
            }),
            network: Some(NetworkOverlay {
                enable: vec!["pypi.org".into()],
            }),
            memory: Some(MemoryOverlay {
                write: vec!["run".into()],
            }),
            spawn: Some(SpawnOverlay {
                max_children: Some(4),
                require_approval_after: Some(2),
            }),
        };

        let compiled = compiler
            .compile_sync("implementer", Some(&overlay))
            .unwrap();

        assert!(compiled.overlay_hash.is_some());
        assert!(compiled.manifest.tools.contains(&"git".to_string()));
        assert!(compiled.manifest.tools.contains(&"python3".to_string()));
        assert_eq!(compiled.manifest.credentials.len(), 2);
        assert!(
            compiled
                .manifest
                .credentials
                .iter()
                .any(|grant| grant.handle == "pypi-publish")
        );
        assert!(
            compiled
                .manifest
                .permissions
                .network_allowlist
                .contains("pypi.org")
        );
        assert_eq!(compiled.manifest.memory_policy.write_scopes.len(), 1);
        assert!(matches!(
            compiled.manifest.memory_policy.write_scopes[0],
            MemoryScope::RunShared
        ));
        assert_eq!(compiled.manifest.permissions.spawn_limits.max_children, 4);
        assert_eq!(
            compiled
                .manifest
                .permissions
                .spawn_limits
                .require_approval_after,
            2
        );
    }

    #[test]
    fn compile_rejects_undeclared_optional_tool() {
        let compiler = ProfileCompiler::new(vec![make_implementer_base()]).unwrap();
        let overlay = ProjectOverlay {
            extends: "implementer".into(),
            tools: Some(ToolsOverlay {
                enable: vec!["malicious-tool".into()],
            }),
            credentials: None,
            network: None,
            memory: None,
            spawn: None,
        };

        let error = compiler
            .compile_sync("implementer", Some(&overlay))
            .unwrap_err()
            .to_string();

        assert!(error.contains("malicious-tool"));
    }

    #[test]
    fn compile_rejects_undeclared_optional_credential() {
        let compiler = ProfileCompiler::new(vec![make_implementer_base()]).unwrap();
        let overlay = ProjectOverlay {
            extends: "implementer".into(),
            tools: None,
            credentials: Some(CredentialsOverlay {
                enable: vec!["aws-root".into()],
            }),
            network: None,
            memory: None,
            spawn: None,
        };

        let error = compiler
            .compile_sync("implementer", Some(&overlay))
            .unwrap_err()
            .to_string();

        assert!(error.contains("aws-root"));
    }

    #[test]
    fn compile_rejects_undeclared_network_host() {
        let compiler = ProfileCompiler::new(vec![make_implementer_base()]).unwrap();
        let overlay = ProjectOverlay {
            extends: "implementer".into(),
            tools: None,
            credentials: None,
            network: Some(NetworkOverlay {
                enable: vec!["evil.example.com".into()],
            }),
            memory: None,
            spawn: None,
        };

        let error = compiler
            .compile_sync("implementer", Some(&overlay))
            .unwrap_err()
            .to_string();

        assert!(error.contains("evil.example.com"));
    }

    #[test]
    fn compile_rejects_spawn_limit_increase() {
        let compiler = ProfileCompiler::new(vec![make_implementer_base()]).unwrap();
        let overlay = ProjectOverlay {
            extends: "implementer".into(),
            tools: None,
            credentials: None,
            network: None,
            memory: None,
            spawn: Some(SpawnOverlay {
                max_children: Some(11),
                require_approval_after: None,
            }),
        };

        let error = compiler
            .compile_sync("implementer", Some(&overlay))
            .unwrap_err()
            .to_string();

        assert!(error.contains("max_children"));
    }

    #[test]
    fn compile_rejects_write_scope_outside_base_policy() {
        let compiler = ProfileCompiler::new(vec![make_implementer_base()]).unwrap();
        let overlay = ProjectOverlay {
            extends: "implementer".into(),
            tools: None,
            credentials: None,
            network: None,
            memory: Some(MemoryOverlay {
                write: vec!["project".into()],
            }),
            spawn: None,
        };

        let error = compiler
            .compile_sync("implementer", Some(&overlay))
            .unwrap_err()
            .to_string();

        assert!(error.contains("project"));
    }

    #[test]
    fn compile_unknown_base_profile_fails() {
        let compiler = ProfileCompiler::new(vec![make_implementer_base()]).unwrap();

        let error = compiler
            .compile_sync("reviewer", None)
            .unwrap_err()
            .to_string();

        assert!(error.contains("reviewer"));
    }

    #[test]
    fn compile_overlay_extends_mismatch_fails() {
        let compiler = ProfileCompiler::new(vec![make_implementer_base()]).unwrap();
        let overlay = ProjectOverlay {
            extends: "security-reviewer".into(),
            tools: None,
            credentials: None,
            network: None,
            memory: None,
            spawn: None,
        };

        let error = compiler
            .compile_sync("implementer", Some(&overlay))
            .unwrap_err()
            .to_string();

        assert!(error.contains("security-reviewer"));
    }

    #[test]
    fn parse_overlay_from_toml_string() {
        let overlay = ProfileCompiler::parse_overlay_str(
            r#"
extends = "implementer"

[tools]
enable = ["python3", "poetry"]

[credentials]
enable = ["pypi-publish"]

[network]
enable = ["pypi.org"]

[memory]
write = ["run"]

[spawn]
max_children = 4
require_approval_after = 2
"#,
        )
        .unwrap();

        assert_eq!(overlay.extends, "implementer");
        assert_eq!(
            overlay.tools.unwrap().enable,
            vec!["python3".to_string(), "poetry".to_string()]
        );
        assert_eq!(overlay.spawn.unwrap().max_children, Some(4));
    }

    #[test]
    fn parse_overlay_rejects_unknown_fields() {
        let result = ProfileCompiler::parse_overlay_str(
            r#"
extends = "implementer"

[hooks]
pre_run = "echo pwned"
"#,
        );

        assert!(result.is_err());
    }

    #[test]
    fn parse_overlays_from_dir_reads_only_toml_files() {
        let temp_dir = TempDir::new().unwrap();
        fs::write(
            temp_dir.path().join("a.toml"),
            r#"
extends = "implementer"

[tools]
enable = ["python3"]
"#,
        )
        .unwrap();
        fs::write(
            temp_dir.path().join("b.toml"),
            r#"
extends = "implementer"

[network]
enable = ["pypi.org"]
"#,
        )
        .unwrap();
        fs::write(temp_dir.path().join("ignore.txt"), "not toml").unwrap();

        let overlays = ProfileCompiler::parse_overlays_from_dir(temp_dir.path()).unwrap();

        assert_eq!(overlays.len(), 2);
        assert_eq!(overlays[0].extends, "implementer");
        assert_eq!(overlays[1].extends, "implementer");
    }

    #[test]
    fn compiled_profile_cache_hit_reuses_overlay_hash_key() {
        let compiler = ProfileCompiler::new(vec![make_implementer_base()]).unwrap();
        let overlay = ProjectOverlay {
            extends: "implementer".into(),
            tools: Some(ToolsOverlay {
                enable: vec!["python3".into()],
            }),
            credentials: None,
            network: None,
            memory: None,
            spawn: Some(SpawnOverlay {
                max_children: Some(3),
                require_approval_after: None,
            }),
        };

        let first = compiler
            .compile_sync("implementer", Some(&overlay))
            .unwrap();
        let second = compiler
            .compile_sync("implementer", Some(&overlay))
            .unwrap();

        assert_eq!(first.overlay_hash, second.overlay_hash);
        assert_eq!(compiler.cache_size(), 1);
        assert_eq!(first.manifest.tools, second.manifest.tools);
        assert_eq!(
            second
                .manifest
                .permissions
                .spawn_limits
                .require_approval_after,
            3
        );

        compiler.invalidate_cache();
        assert_eq!(compiler.cache_size(), 0);
    }

    #[test]
    fn new_rejects_duplicate_base_profile_names() {
        let error =
            match ProfileCompiler::new(vec![make_implementer_base(), make_implementer_base()]) {
                Ok(_) => panic!("expected duplicate trusted base profile names to fail"),
                Err(error) => error.to_string(),
            };

        assert!(error.contains("duplicate trusted base profile"));
    }
}
