use anyhow::Context;
use serde::{Deserialize, Serialize};

const GITHUB_DEVICE_CODE_URL: &str = "https://github.com/login/device/code";
const GITHUB_TOKEN_URL: &str = "https://github.com/login/oauth/access_token";
const GITHUB_USER_REPOS_URL: &str = "https://api.github.com/user/repos";

/// Response from GitHub's device code endpoint.
#[derive(Debug, Deserialize, Serialize)]
pub struct DeviceCodeResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    pub interval: u64,
}

/// Response from GitHub's token polling endpoint.
#[derive(Debug, Deserialize)]
pub struct TokenResponse {
    pub access_token: Option<String>,
    pub token_type: Option<String>,
    pub scope: Option<String>,
    pub error: Option<String>,
}

/// A GitHub repository (subset of fields we care about).
#[derive(Debug, Serialize, Deserialize)]
pub struct GitHubRepo {
    pub full_name: String,
    pub name: String,
    pub private: bool,
    pub html_url: String,
    pub clone_url: String,
    pub description: Option<String>,
    pub default_branch: String,
}

/// A GitHub issue (subset of fields).
#[derive(Debug, Serialize, Deserialize)]
pub struct GitHubIssue {
    pub number: i64,
    pub title: String,
    pub body: Option<String>,
    pub state: String,
    pub html_url: String,
    /// Pull requests also come through the issues endpoint; filter them out.
    pub pull_request: Option<serde_json::Value>,
}

/// Known GitHub token prefixes.
/// See: https://github.blog/2021-04-05-behind-githubs-new-authentication-token-formats/
const GITHUB_TOKEN_PREFIXES: &[&str] = &[
    "ghp_",        // Personal access tokens (classic)
    "github_pat_", // Fine-grained personal access tokens
    "gho_",        // OAuth access tokens
    "ghu_",        // GitHub App user-to-server tokens
    "ghs_",        // GitHub App server-to-server tokens
    "ghr_",        // GitHub App refresh tokens
];

/// Validate that a string looks like a valid GitHub token based on its prefix.
///
/// This performs a format check only — it does not verify the token is active
/// or has appropriate scopes. Use this for fast client-side validation before
/// making network calls.
pub fn is_valid_github_token(token: &str) -> bool {
    if token.is_empty() {
        return false;
    }
    GITHUB_TOKEN_PREFIXES
        .iter()
        .any(|prefix| token.starts_with(prefix))
}

/// Parse the `owner/repo` slug from a GitHub URL.
///
/// Handles both HTTPS and token-embedded URLs:
/// - `https://github.com/owner/repo`
/// - `https://github.com/owner/repo.git`
/// - `https://x-access-token:TOKEN@github.com/owner/repo.git`
pub fn parse_owner_repo_from_url(url: &str) -> Option<String> {
    // Strip the token-embedded prefix if present
    let path = if let Some(rest) = url.strip_prefix("https://") {
        // Could be "x-access-token:TOKEN@github.com/owner/repo" or "github.com/owner/repo"
        if let Some(after_at) = rest.strip_prefix("x-access-token:") {
            // Find the '@' separator
            after_at.find('@').map(|idx| &after_at[idx + 1..])
        } else {
            Some(rest)
        }
    } else {
        None
    }?;

    // Now path should be "github.com/owner/repo[.git]"
    let repo_path = path.strip_prefix("github.com/")?;
    let repo_path = repo_path.strip_suffix(".git").unwrap_or(repo_path);

    // Validate it looks like "owner/repo" (exactly two segments)
    let parts: Vec<&str> = repo_path.split('/').collect();
    if parts.len() == 2 && !parts[0].is_empty() && !parts[1].is_empty() {
        Some(format!("{}/{}", parts[0], parts[1]))
    } else {
        None
    }
}

/// Start the device flow — returns device code + user code for the user to enter.
pub async fn request_device_code(client_id: &str) -> anyhow::Result<DeviceCodeResponse> {
    let client = reqwest::Client::new();
    let resp = client
        .post(GITHUB_DEVICE_CODE_URL)
        .header("Accept", "application/json")
        .form(&[("client_id", client_id), ("scope", "repo")])
        .send()
        .await
        .context("Failed to send device code request to GitHub")?;

    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        anyhow::bail!(
            "GitHub rejected the OAuth client ID. Ensure GITHUB_CLIENT_ID is set to a valid \
             GitHub OAuth App with Device Flow enabled. \
             Create one at https://github.com/settings/developers"
        );
    }

    let resp = resp
        .error_for_status()
        .context("GitHub device code endpoint returned error status")?;
    resp.json::<DeviceCodeResponse>()
        .await
        .context("Failed to parse device code response from GitHub")
}

/// Poll GitHub for the access token. Returns Ok(Some(token)) when authorized,
/// Ok(None) when still pending, or Err on actual errors.
pub async fn poll_for_token(client_id: &str, device_code: &str) -> anyhow::Result<Option<String>> {
    let client = reqwest::Client::new();
    let resp = client
        .post(GITHUB_TOKEN_URL)
        .header("Accept", "application/json")
        .form(&[
            ("client_id", client_id),
            ("device_code", device_code),
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
        ])
        .send()
        .await
        .context("Failed to send token poll request to GitHub")?
        .json::<TokenResponse>()
        .await
        .context("Failed to parse token poll response from GitHub")?;

    if let Some(token) = resp.access_token {
        return Ok(Some(token));
    }

    match resp.error.as_deref() {
        Some("authorization_pending") | Some("slow_down") => Ok(None),
        Some(err) => anyhow::bail!("GitHub auth error: {}", err),
        None => anyhow::bail!("Unexpected response from GitHub"),
    }
}

/// List open issues for a repository (excludes pull requests).
/// Paginates through all pages automatically.
pub async fn list_issues(token: &str, owner_repo: &str) -> anyhow::Result<Vec<GitHubIssue>> {
    let client = reqwest::Client::new();
    let url = format!("https://api.github.com/repos/{}/issues", owner_repo);
    let mut all_issues = Vec::new();
    let mut page = 1u32;

    loop {
        let resp: Vec<GitHubIssue> = client
            .get(&url)
            .header("Authorization", format!("Bearer {}", token))
            .header("User-Agent", "forge-factory")
            .query(&[
                ("state", "open"),
                ("per_page", "100"),
                ("page", &page.to_string()),
            ])
            .send()
            .await
            .context("Failed to send issues request to GitHub")?
            .error_for_status()
            .context("GitHub issues API returned error status")?
            .json()
            .await
            .context("Failed to parse issues response from GitHub")?;

        let count = resp.len();
        // Filter out pull requests (they have a pull_request key)
        all_issues.extend(resp.into_iter().filter(|i| i.pull_request.is_none()));

        if count < 100 {
            break; // Last page
        }
        page += 1;
    }

    Ok(all_issues)
}

/// List repos accessible to the authenticated user.
pub async fn list_repos(token: &str, page: u32, per_page: u32) -> anyhow::Result<Vec<GitHubRepo>> {
    let client = reqwest::Client::new();
    let repos = client
        .get(GITHUB_USER_REPOS_URL)
        .header("Authorization", format!("Bearer {}", token))
        .header("User-Agent", "forge-factory")
        .query(&[
            ("sort", "updated"),
            ("per_page", &per_page.to_string()),
            ("page", &page.to_string()),
        ])
        .send()
        .await
        .context("Failed to send repos request to GitHub")?
        .error_for_status()
        .context("GitHub repos API returned error status")?
        .json::<Vec<GitHubRepo>>()
        .await
        .context("Failed to parse repos response from GitHub")?;
    Ok(repos)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── is_valid_github_token ────────────────────────────────────────

    #[test]
    fn test_valid_personal_access_token_classic() {
        assert!(is_valid_github_token("ghp_abc123def456"));
    }

    #[test]
    fn test_valid_fine_grained_pat() {
        assert!(is_valid_github_token("github_pat_abc123def456"));
    }

    #[test]
    fn test_valid_oauth_token() {
        assert!(is_valid_github_token("gho_abc123"));
    }

    #[test]
    fn test_valid_user_to_server_token() {
        assert!(is_valid_github_token("ghu_xyz789"));
    }

    #[test]
    fn test_valid_server_to_server_token() {
        assert!(is_valid_github_token("ghs_xyz789"));
    }

    #[test]
    fn test_valid_refresh_token() {
        assert!(is_valid_github_token("ghr_refreshtoken123"));
    }

    #[test]
    fn test_empty_token_is_invalid() {
        assert!(!is_valid_github_token(""));
    }

    #[test]
    fn test_random_string_is_invalid() {
        assert!(!is_valid_github_token("not-a-token"));
    }

    #[test]
    fn test_prefix_only_is_valid() {
        // A bare prefix with nothing after it still starts_with the prefix
        assert!(is_valid_github_token("ghp_"));
    }

    #[test]
    fn test_wrong_prefix_is_invalid() {
        assert!(!is_valid_github_token("ghx_abc123"));
    }

    #[test]
    fn test_uppercase_prefix_is_invalid() {
        assert!(!is_valid_github_token("GHP_abc123"));
    }

    #[test]
    fn test_whitespace_only_is_invalid() {
        assert!(!is_valid_github_token("   "));
    }

    #[test]
    fn test_token_with_leading_space_is_invalid() {
        assert!(!is_valid_github_token(" ghp_abc123"));
    }

    // ── parse_owner_repo_from_url ────────────────────────────────────

    #[test]
    fn test_parse_simple_https_url() {
        assert_eq!(
            parse_owner_repo_from_url("https://github.com/owner/repo"),
            Some("owner/repo".to_string())
        );
    }

    #[test]
    fn test_parse_https_url_with_git_suffix() {
        assert_eq!(
            parse_owner_repo_from_url("https://github.com/owner/repo.git"),
            Some("owner/repo".to_string())
        );
    }

    #[test]
    fn test_parse_token_embedded_url() {
        assert_eq!(
            parse_owner_repo_from_url(
                "https://x-access-token:ghp_abc123@github.com/owner/repo.git"
            ),
            Some("owner/repo".to_string())
        );
    }

    #[test]
    fn test_parse_token_embedded_url_no_git_suffix() {
        assert_eq!(
            parse_owner_repo_from_url("https://x-access-token:ghp_abc123@github.com/owner/repo"),
            Some("owner/repo".to_string())
        );
    }

    #[test]
    fn test_parse_url_missing_repo() {
        assert_eq!(parse_owner_repo_from_url("https://github.com/owner"), None);
    }

    #[test]
    fn test_parse_url_too_many_segments() {
        assert_eq!(
            parse_owner_repo_from_url("https://github.com/owner/repo/extra"),
            None
        );
    }

    #[test]
    fn test_parse_non_github_url() {
        assert_eq!(
            parse_owner_repo_from_url("https://gitlab.com/owner/repo"),
            None
        );
    }

    #[test]
    fn test_parse_empty_string() {
        assert_eq!(parse_owner_repo_from_url(""), None);
    }

    #[test]
    fn test_parse_ssh_url_returns_none() {
        // SSH-style URLs are not supported by this function
        assert_eq!(
            parse_owner_repo_from_url("git@github.com:owner/repo.git"),
            None
        );
    }

    // ── TokenResponse deserialization ────────────────────────────────

    #[test]
    fn test_token_response_with_access_token() {
        let json = r#"{"access_token":"ghp_abc123","token_type":"bearer","scope":"repo"}"#;
        let resp: TokenResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.access_token.as_deref(), Some("ghp_abc123"));
        assert_eq!(resp.token_type.as_deref(), Some("bearer"));
        assert_eq!(resp.scope.as_deref(), Some("repo"));
        assert!(resp.error.is_none());
    }

    #[test]
    fn test_token_response_pending() {
        let json = r#"{"error":"authorization_pending"}"#;
        let resp: TokenResponse = serde_json::from_str(json).unwrap();
        assert!(resp.access_token.is_none());
        assert_eq!(resp.error.as_deref(), Some("authorization_pending"));
    }

    #[test]
    fn test_token_response_slow_down() {
        let json = r#"{"error":"slow_down"}"#;
        let resp: TokenResponse = serde_json::from_str(json).unwrap();
        assert!(resp.access_token.is_none());
        assert_eq!(resp.error.as_deref(), Some("slow_down"));
    }

    #[test]
    fn test_token_response_denied() {
        let json = r#"{"error":"access_denied"}"#;
        let resp: TokenResponse = serde_json::from_str(json).unwrap();
        assert!(resp.access_token.is_none());
        assert_eq!(resp.error.as_deref(), Some("access_denied"));
    }

    // ── DeviceCodeResponse deserialization ────────────────────────────

    #[test]
    fn test_device_code_response_deserialize() {
        let json = r#"{
            "device_code": "dc_abc123",
            "user_code": "ABCD-1234",
            "verification_uri": "https://github.com/login/device",
            "expires_in": 900,
            "interval": 5
        }"#;
        let resp: DeviceCodeResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.device_code, "dc_abc123");
        assert_eq!(resp.user_code, "ABCD-1234");
        assert_eq!(resp.verification_uri, "https://github.com/login/device");
        assert_eq!(resp.expires_in, 900);
        assert_eq!(resp.interval, 5);
    }

    #[test]
    fn test_device_code_response_roundtrip() {
        let original = DeviceCodeResponse {
            device_code: "dc_test".to_string(),
            user_code: "TEST-CODE".to_string(),
            verification_uri: "https://github.com/login/device".to_string(),
            expires_in: 600,
            interval: 10,
        };
        let json = serde_json::to_string(&original).unwrap();
        let deserialized: DeviceCodeResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.device_code, original.device_code);
        assert_eq!(deserialized.user_code, original.user_code);
        assert_eq!(deserialized.expires_in, original.expires_in);
    }

    // ── GitHubIssue deserialization ──────────────────────────────────

    #[test]
    fn test_github_issue_deserialize_regular_issue() {
        let json = r#"{
            "number": 42,
            "title": "Bug: something broken",
            "body": "Steps to reproduce...",
            "state": "open",
            "html_url": "https://github.com/owner/repo/issues/42"
        }"#;
        let issue: GitHubIssue = serde_json::from_str(json).unwrap();
        assert_eq!(issue.number, 42);
        assert_eq!(issue.title, "Bug: something broken");
        assert_eq!(issue.body.as_deref(), Some("Steps to reproduce..."));
        assert_eq!(issue.state, "open");
        assert!(issue.pull_request.is_none());
    }

    #[test]
    fn test_github_issue_deserialize_pull_request() {
        let json = r#"{
            "number": 10,
            "title": "Add feature",
            "body": null,
            "state": "open",
            "html_url": "https://github.com/owner/repo/pull/10",
            "pull_request": {"url": "https://api.github.com/repos/owner/repo/pulls/10"}
        }"#;
        let issue: GitHubIssue = serde_json::from_str(json).unwrap();
        assert_eq!(issue.number, 10);
        assert!(issue.pull_request.is_some());
        assert!(issue.body.is_none());
    }

    #[test]
    fn test_github_issue_filter_prs() {
        let issues_json = r#"[
            {"number": 1, "title": "Real issue", "body": null, "state": "open", "html_url": "https://github.com/o/r/issues/1"},
            {"number": 2, "title": "PR", "body": null, "state": "open", "html_url": "https://github.com/o/r/pull/2", "pull_request": {"url": "..."}}
        ]"#;
        let issues: Vec<GitHubIssue> = serde_json::from_str(issues_json).unwrap();
        let filtered: Vec<_> = issues
            .into_iter()
            .filter(|i| i.pull_request.is_none())
            .collect();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].number, 1);
    }

    // ── GitHubRepo deserialization ───────────────────────────────────

    #[test]
    fn test_github_repo_deserialize() {
        let json = r#"{
            "full_name": "owner/repo",
            "name": "repo",
            "private": true,
            "html_url": "https://github.com/owner/repo",
            "clone_url": "https://github.com/owner/repo.git",
            "description": "A test repo",
            "default_branch": "main"
        }"#;
        let repo: GitHubRepo = serde_json::from_str(json).unwrap();
        assert_eq!(repo.full_name, "owner/repo");
        assert_eq!(repo.name, "repo");
        assert!(repo.private);
        assert_eq!(repo.clone_url, "https://github.com/owner/repo.git");
        assert_eq!(repo.description.as_deref(), Some("A test repo"));
        assert_eq!(repo.default_branch, "main");
    }

    #[test]
    fn test_github_repo_null_description() {
        let json = r#"{
            "full_name": "owner/repo",
            "name": "repo",
            "private": false,
            "html_url": "https://github.com/owner/repo",
            "clone_url": "https://github.com/owner/repo.git",
            "description": null,
            "default_branch": "develop"
        }"#;
        let repo: GitHubRepo = serde_json::from_str(json).unwrap();
        assert!(!repo.private);
        assert!(repo.description.is_none());
        assert_eq!(repo.default_branch, "develop");
    }

    // ── Constants ────────────────────────────────────────────────────

    #[test]
    fn test_github_token_prefixes_are_non_empty() {
        assert!(!GITHUB_TOKEN_PREFIXES.is_empty());
        for prefix in GITHUB_TOKEN_PREFIXES {
            assert!(!prefix.is_empty(), "Token prefix should not be empty");
            assert!(
                prefix.ends_with('_'),
                "Token prefix should end with underscore: {}",
                prefix
            );
        }
    }

    #[test]
    fn test_all_six_known_prefixes_present() {
        let expected = vec!["ghp_", "github_pat_", "gho_", "ghu_", "ghs_", "ghr_"];
        for prefix in &expected {
            assert!(
                GITHUB_TOKEN_PREFIXES.contains(prefix),
                "Missing expected prefix: {}",
                prefix
            );
        }
        assert_eq!(GITHUB_TOKEN_PREFIXES.len(), expected.len());
    }
}
