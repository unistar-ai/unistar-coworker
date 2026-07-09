use chrono::Utc;

use crate::store::Store;

pub async fn build_handoff_markdown(store: &dyn Store) -> crate::error::Result<String> {
    let digest = store.latest_digest().await?;
    let approvals = store.list_pending_approvals().await?;

    let mut body = String::from("# On-call handoff\n\n");
    body.push_str(&format!(
        "Generated: {}\n\n",
        Utc::now().format("%Y-%m-%d %H:%M UTC")
    ));

    body.push_str("## Latest digest\n\n");
    match digest {
        Some(d) => {
            body.push_str(&format!(
                "- Date: {}\n- Summary: {} need attention, {} flaky, {} policy, {} ignorable\n",
                d.date,
                d.summary.needs_attention,
                d.summary.flaky_candidates,
                d.summary.policy_gates,
                d.summary.ignorable,
            ));
        }
        None => body.push_str("_No digest in store yet._\n"),
    }

    body.push_str("\n## Pending approvals\n\n");
    if approvals.is_empty() {
        body.push_str("_None._\n");
    } else {
        for a in &approvals {
            body.push_str(&format!("- {:?}: {}\n", a.kind, a.description));
        }
    }

    Ok(body)
}
