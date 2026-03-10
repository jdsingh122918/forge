use crate::council::types::ReviewResult;
use crate::phase::Phase;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const LABEL_PAIRS: [(&str, &str); 13] = [
    ("Alpha", "Bravo"),
    ("Charlie", "Delta"),
    ("Echo", "Foxtrot"),
    ("Golf", "Hotel"),
    ("India", "Juliet"),
    ("Kilo", "Lima"),
    ("Mike", "November"),
    ("Oscar", "Papa"),
    ("Quebec", "Romeo"),
    ("Sierra", "Tango"),
    ("Uniform", "Victor"),
    ("Whiskey", "Xray"),
    ("Yankee", "Zulu"),
];

const DEFAULT_CHAIRMAN_RETRY_BUDGET: u32 = 3;

static NEXT_LABEL_PAIR_INDEX: AtomicUsize = AtomicUsize::new(0);

pub fn generate_label_pair() -> (String, String) {
    let time_seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.subsec_nanos() as usize)
        .unwrap_or(0);
    let index = NEXT_LABEL_PAIR_INDEX
        .fetch_add(1, Ordering::Relaxed)
        .wrapping_add(time_seed)
        % LABEL_PAIRS.len();
    let (first, second) = LABEL_PAIRS[index];
    (first.to_string(), second.to_string())
}

pub fn anonymize_label<'a>(worker_name: &str, label_map: &'a HashMap<String, String>) -> &'a str {
    label_map
        .get(worker_name)
        .map(String::as_str)
        .unwrap_or_else(|| panic!("missing anonymized label for worker `{worker_name}`"))
}

pub fn review_prompt(phase: &Phase, diff: &str, candidate_label: &str) -> String {
    let candidate_label = candidate_display_label(candidate_label);

    format!(
        "You are performing Stage 2 anonymized peer review for Forge's council engine.\n\
         Review only the diff shown below for {candidate_label}. Do not guess which model produced it.\n\n\
         Phase context:\n\
         - Phase: {} {}\n\
         - Promise tag: {}\n\
         - Iteration budget: {}\n\
         - Reasoning: {}\n\n\
         Evaluate correctness, completeness, style, and performance. Cite specific file paths and line references in the issues list when possible.\n\
         Return JSON only. Do not return Markdown.\n\
         Required JSON shape:\n\
         {{\n\
           \"candidate_label\": \"{candidate_label}\",\n\
           \"verdict\": \"approve\" | \"request_changes\" | \"abstain\",\n\
           \"request_changes_reason\": \"string or null\",\n\
           \"scores\": {{\n\
             \"correctness\": 0.0,\n\
             \"completeness\": 0.0,\n\
             \"style\": 0.0,\n\
             \"performance\": 0.0,\n\
             \"overall\": 0.0\n\
           }},\n\
           \"issues\": [\"path:line concise issue\"],\n\
           \"summary\": \"short overall assessment\"\n\
         }}\n\n\
         Diff for {candidate_label}:\n\
         ```diff\n\
         {diff}\n\
         ```",
        phase.number, phase.name, phase.promise, phase.budget, phase.reasoning
    )
}

pub fn chairman_synthesis_prompt(
    phase: &Phase,
    diffs: &[(&str, &str)],
    reviews: &[ReviewResult],
) -> String {
    let diff_sections = diffs
        .iter()
        .map(|(label, diff)| {
            format!(
                "{} diff:\n```diff\n{}\n```",
                candidate_display_label(label),
                diff
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    let review_sections = if reviews.is_empty() {
        "No peer review feedback was provided.".to_string()
    } else {
        reviews
            .iter()
            .map(format_review_feedback)
            .collect::<Vec<_>>()
            .join("\n\n")
    };

    format!(
        "You are the chairman for Stage 3 council synthesis in Forge.\n\
         Produce the best merged result for the phase below. Do not mechanically union both diffs; resolve disagreements intentionally.\n\n\
         Phase context:\n\
         - Phase: {} {}\n\
         - Promise tag: {}\n\
         - Iteration budget: {}\n\
         - Reasoning: {}\n\
         - Chairman retry budget: {} total attempts. If `git apply` fails, a retry prompt will include the failure output.\n\n\
         Inputs:\n\
         {diff_sections}\n\n\
         Peer review feedback:\n\
         {review_sections}\n\n\
         Output requirements:\n\
         - Emit only a git format patch / unified diff that `git apply` can consume.\n\
         - Do not include prose, Markdown fences, or explanations outside the patch.\n\
         - Preserve the strongest parts of each candidate while fixing issues called out in review.\n\
         - The result must apply cleanly against the baseline.",
        phase.number,
        phase.name,
        phase.promise,
        phase.budget,
        phase.reasoning,
        DEFAULT_CHAIRMAN_RETRY_BUDGET
    )
}

fn candidate_display_label(label: &str) -> String {
    if label.starts_with("Candidate ") {
        label.to_string()
    } else {
        format!("Candidate {label}")
    }
}

fn format_review_feedback(review: &ReviewResult) -> String {
    let request_changes_reason = match &review.verdict {
        crate::council::types::ReviewVerdict::Approve => "approve".to_string(),
        crate::council::types::ReviewVerdict::RequestChanges(reason) => {
            format!("request_changes ({reason})")
        }
        crate::council::types::ReviewVerdict::Abstain => "abstain".to_string(),
    };
    let issues = if review.issues.is_empty() {
        "- none".to_string()
    } else {
        review
            .issues
            .iter()
            .map(|issue| format!("- {issue}"))
            .collect::<Vec<_>>()
            .join("\n")
    };

    format!(
        "{} review:\n\
         - Verdict: {}\n\
         - Scores: correctness={}, completeness={}, style={}, performance={}, overall={}\n\
         - Summary: {}\n\
         - Issues:\n{}",
        candidate_display_label(&review.candidate_label),
        request_changes_reason,
        review.scores.correctness,
        review.scores.completeness,
        review.scores.style,
        review.scores.performance,
        review.scores.overall,
        review.summary,
        issues
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::council::types::{ReviewScores, ReviewVerdict};
    use std::collections::{HashMap, HashSet};
    use std::time::Duration;

    fn test_phase() -> Phase {
        Phase::new(
            "08",
            "Prompt templates and label anonymization",
            "DONE",
            2,
            "Create anonymized review prompts and chairman synthesis prompts.",
            vec![],
        )
    }

    fn test_review(label: &str, summary: &str) -> ReviewResult {
        ReviewResult {
            reviewer_name: "reviewer".to_string(),
            candidate_label: label.to_string(),
            verdict: ReviewVerdict::RequestChanges("Needs cleanup".to_string()),
            scores: ReviewScores {
                correctness: 0.8,
                completeness: 0.7,
                style: 0.9,
                performance: 0.6,
                overall: 0.75,
            },
            issues: vec!["src/lib.rs:12 missing context".to_string()],
            summary: summary.to_string(),
            duration: Duration::from_secs(1),
        }
    }

    #[test]
    fn test_generate_label_pair_returns_two_different_labels() {
        let (a, b) = generate_label_pair();
        assert_ne!(a, b);
    }

    #[test]
    fn test_generate_label_pair_labels_from_known_pool() {
        let known_labels: HashSet<&str> = LABEL_PAIRS.iter().flat_map(|(a, b)| [*a, *b]).collect();

        for _ in 0..100 {
            let (a, b) = generate_label_pair();
            assert!(known_labels.contains(a.as_str()));
            assert!(known_labels.contains(b.as_str()));
        }
    }

    #[test]
    fn test_anonymize_label_maps_correctly() {
        let mut map = HashMap::new();
        map.insert("claude".to_string(), "Alpha".to_string());
        assert_eq!(anonymize_label("claude", &map), "Alpha");
    }

    #[test]
    fn test_review_prompt_contains_phase_name() {
        let prompt = review_prompt(
            &test_phase(),
            "diff --git a/src/lib.rs b/src/lib.rs",
            "Alpha",
        );

        assert!(prompt.contains("Prompt templates and label anonymization"));
    }

    #[test]
    fn test_review_prompt_contains_candidate_label() {
        let prompt = review_prompt(
            &test_phase(),
            "diff --git a/src/lib.rs b/src/lib.rs",
            "Alpha",
        );

        assert!(prompt.contains("Candidate Alpha"));
    }

    #[test]
    fn test_review_prompt_contains_diff_content() {
        let diff = "diff --git a/src/lib.rs b/src/lib.rs\n+pub mod prompts;";
        let prompt = review_prompt(&test_phase(), diff, "Alpha");

        assert!(prompt.contains(diff));
    }

    #[test]
    fn test_review_prompt_does_not_contain_worker_name() {
        let prompt = review_prompt(
            &test_phase(),
            "diff --git a/src/lib.rs b/src/lib.rs",
            "Alpha",
        );

        assert!(!prompt.contains("claude"));
        assert!(!prompt.contains("codex"));
    }

    #[test]
    fn test_review_prompt_requests_structured_json() {
        let prompt = review_prompt(
            &test_phase(),
            "diff --git a/src/lib.rs b/src/lib.rs",
            "Alpha",
        );

        assert!(prompt.contains("JSON"));
        assert!(prompt.contains("scores"));
    }

    #[test]
    fn test_chairman_synthesis_prompt_contains_both_diffs() {
        let prompt = chairman_synthesis_prompt(
            &test_phase(),
            &[
                ("Alpha", "diff --git a/src/a.rs b/src/a.rs\n+alpha"),
                ("Bravo", "diff --git a/src/b.rs b/src/b.rs\n+bravo"),
            ],
            &[],
        );

        assert!(prompt.contains("diff --git a/src/a.rs b/src/a.rs\n+alpha"));
        assert!(prompt.contains("diff --git a/src/b.rs b/src/b.rs\n+bravo"));
    }

    #[test]
    fn test_chairman_synthesis_prompt_contains_review_feedback() {
        let prompt = chairman_synthesis_prompt(
            &test_phase(),
            &[
                ("Alpha", "diff --git a/src/a.rs b/src/a.rs\n+alpha"),
                ("Bravo", "diff --git a/src/b.rs b/src/b.rs\n+bravo"),
            ],
            &[
                test_review("Candidate Alpha", "Candidate Alpha keeps the API smaller."),
                test_review(
                    "Candidate Bravo",
                    "Candidate Bravo adds stronger error handling.",
                ),
            ],
        );

        assert!(prompt.contains("Candidate Alpha keeps the API smaller."));
        assert!(prompt.contains("Candidate Bravo adds stronger error handling."));
    }

    #[test]
    fn test_chairman_synthesis_prompt_requests_git_format_patch() {
        let prompt = chairman_synthesis_prompt(
            &test_phase(),
            &[("Alpha", "diff --git a/src/a.rs b/src/a.rs\n+alpha")],
            &[],
        );

        assert!(prompt.contains("git-format patch") || prompt.contains("git format patch"));
    }

    #[test]
    fn test_chairman_synthesis_prompt_mentions_retry_budget() {
        let prompt = chairman_synthesis_prompt(
            &test_phase(),
            &[("Alpha", "diff --git a/src/a.rs b/src/a.rs\n+alpha")],
            &[],
        );

        assert!(prompt.to_lowercase().contains("retry budget"));
    }
}
