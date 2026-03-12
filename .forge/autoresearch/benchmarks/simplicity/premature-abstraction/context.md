# Provider Abstraction Layer — Premature Abstraction

This module defines a trait-based abstraction layer for version control, pull requests, and CI operations. Four traits are defined:
- `VersionControl` (8 methods)
- `PullRequestProvider` (5 methods)
- `CiProvider` (3 methods)
- `ProviderFactory` (3 methods)

Each trait has exactly ONE implementation:
- `GitVcs` implements `VersionControl`
- `GitHubPrProvider` implements `PullRequestProvider`
- `GitHubActions` implements `CiProvider`
- `DefaultProviderFactory` implements `ProviderFactory`

There are no plans to support SVN, Mercurial, GitLab, or other providers. The traits add indirection (dynamic dispatch, Box<dyn>), increase code volume, and make navigation harder — all for polymorphism that will never be used.

The simpler alternative: use concrete types directly. If a second provider is ever needed, extract the trait at that time (YAGNI principle).
