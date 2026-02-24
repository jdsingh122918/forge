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

/// Start the device flow — returns device code + user code for the user to enter.
pub async fn request_device_code(client_id: &str) -> anyhow::Result<DeviceCodeResponse> {
    let client = reqwest::Client::new();
    let resp = client
        .post(GITHUB_DEVICE_CODE_URL)
        .header("Accept", "application/json")
        .form(&[("client_id", client_id), ("scope", "repo")])
        .send()
        .await?;

    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        anyhow::bail!(
            "GitHub rejected the OAuth client ID. Ensure GITHUB_CLIENT_ID is set to a valid \
             GitHub OAuth App with Device Flow enabled. \
             Create one at https://github.com/settings/developers"
        );
    }

    let resp = resp.error_for_status()?;
    Ok(resp.json::<DeviceCodeResponse>().await?)
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
        .await?
        .json::<TokenResponse>()
        .await?;

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
            .await?
            .error_for_status()?
            .json()
            .await?;

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
        .await?
        .error_for_status()?
        .json::<Vec<GitHubRepo>>()
        .await?;
    Ok(repos)
}
