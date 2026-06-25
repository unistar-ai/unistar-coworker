//! Parse PR references from digest Markdown for Dashboard → PRs navigation.

/// All `(repo, pr_number)` pairs in document order.
pub fn extract_pr_refs_from_digest(body: &str, fallback_repo: &str) -> Vec<(String, u32)> {
    let mut current_repo: Option<String> = None;
    let mut out = Vec::new();
    for line in body.lines() {
        if let Some(r) = line.strip_prefix("### ") {
            current_repo = Some(r.trim().to_string());
            continue;
        }
        if let Some(pr) = parse_pr_from_line(line, current_repo.as_deref(), fallback_repo) {
            if out
                .last()
                .map(|(r, n)| r != &pr.0 || n != &pr.1)
                .unwrap_or(true)
            {
                out.push(pr);
            }
        }
    }
    out
}

/// PR on or after `line_idx` in the source Markdown; falls back to the first ref.
pub fn pr_ref_at_source_line(
    body: &str,
    line_idx: usize,
    fallback_repo: &str,
) -> Option<(String, u32)> {
    let mut current_repo: Option<String> = None;
    let mut first: Option<(String, u32)> = None;
    for (i, line) in body.lines().enumerate() {
        if let Some(r) = line.strip_prefix("### ") {
            current_repo = Some(r.trim().to_string());
        }
        let pr = parse_pr_from_line(line, current_repo.as_deref(), fallback_repo);
        if let Some(ref p) = pr {
            if first.is_none() {
                first = Some(p.clone());
            }
            if i >= line_idx {
                return pr;
            }
        }
    }
    extract_pr_refs_from_digest(body, fallback_repo)
        .into_iter()
        .next()
        .or(first)
}

fn parse_pr_from_line(
    line: &str,
    repo: Option<&str>,
    fallback_repo: &str,
) -> Option<(String, u32)> {
    let trimmed = line.trim();
    if !(trimmed.starts_with('-') || trimmed.starts_with('*')) {
        return None;
    }
    if let Some(url_repo) = repo_from_github_pull_url(trimmed) {
        if let Some(num) = pr_number_from_github_pull_url(trimmed) {
            return Some((url_repo, num));
        }
    }
    let body = trimmed
        .trim_start_matches('-')
        .trim_start_matches('*')
        .trim();
    if let Some(rest) = body.strip_prefix("[#") {
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        let num: u32 = digits.parse().ok()?;
        let r = repo
            .map(str::to_string)
            .unwrap_or_else(|| fallback_repo.to_string());
        return Some((r, num));
    }
    if let Some(rest) = body.strip_prefix('#') {
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        let num: u32 = digits.parse().ok()?;
        let r = repo
            .map(str::to_string)
            .unwrap_or_else(|| fallback_repo.to_string());
        return Some((r, num));
    }
    None
}

fn repo_from_github_pull_url(s: &str) -> Option<String> {
    let marker = "github.com/";
    let i = s.find(marker)?;
    let rest = &s[i + marker.len()..];
    let slash = rest.find('/')?;
    let owner = &rest[..slash];
    let rest = &rest[slash + 1..];
    let slash2 = rest.find('/')?;
    let name = &rest[..slash2];
    if owner.is_empty() || name.is_empty() {
        return None;
    }
    Some(format!("{owner}/{name}"))
}

fn pr_number_from_github_pull_url(s: &str) -> Option<u32> {
    let marker = "/pull/";
    let i = s.find(marker)?;
    let rest = &s[i + marker.len()..];
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_pr_from_hash_bullet() {
        let body = "## Needs attention\n\n### acme/widget\n\n- #42 fix CI\n";
        let refs = extract_pr_refs_from_digest(body, "fallback/r");
        assert_eq!(refs, vec![("acme/widget".into(), 42)]);
    }

    #[test]
    fn extracts_pr_from_markdown_link() {
        let body = "- [#99 docs](https://github.com/acme/widget/pull/99) (@alice)\n";
        let refs = extract_pr_refs_from_digest(body, "fallback/r");
        assert_eq!(refs, vec![("acme/widget".into(), 99)]);
    }

    #[test]
    fn pr_ref_at_line_skips_earlier_rows() {
        let body = "### acme/widget\n\n- #1 ok\n- #2 broken\n";
        assert_eq!(
            pr_ref_at_source_line(body, 3, "x/y"),
            Some(("acme/widget".into(), 2))
        );
    }
}
