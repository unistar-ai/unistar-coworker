use crate::store::{Store, Transcript};

/// Build a few-shot prefix from recent successful triage transcripts.
pub async fn few_shot_prefix(store: &dyn Store, limit: usize) -> String {
    let transcripts = match store.list_transcripts(limit).await {
        Ok(t) => t,
        Err(_) => return String::new(),
    };
    if transcripts.is_empty() {
        return String::new();
    }
    let mut out = String::from("## Playbook examples (prior triage)\n\n");
    for t in transcripts {
        out.push_str(&format!(
            "- {}/#{} kind={} verdict={}\n  excerpt: {}\n",
            t.repo,
            t.pr_number,
            t.kind,
            t.verdict,
            t.turns_json.chars().take(200).collect::<String>()
        ));
    }
    out.push('\n');
    out
}

pub fn transcript_from_triage(
    repo: &str,
    pr_number: u32,
    kind: &str,
    verdict: &str,
    turns: &str,
) -> Transcript {
    Transcript {
        id: uuid::Uuid::new_v4(),
        repo: repo.to_string(),
        pr_number,
        kind: kind.to_string(),
        turns_json: turns.to_string(),
        verdict: verdict.to_string(),
        created_at: chrono::Utc::now(),
    }
}
