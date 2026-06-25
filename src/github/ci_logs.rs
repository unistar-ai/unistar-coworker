use regex::Regex;
use std::sync::OnceLock;

use super::ci_common::{self, RunJob};
use super::exec::GhExec;
use crate::error::{CoworkerError, Result};

pub const ERR_BUDGET: usize = 6_000;
pub const FALLBACK_TAIL: usize = 4_000;
const ERR_CONTEXT: usize = 4;
const ERR_BLOCK_MAX_LINES: usize = 12;

fn err_line_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)(\berror\b|\bfailed\b|\bfailure\b|\bpanic\b|\bfatal\b|exception|traceback|assert|\bundefined\b|cannot |not found|exit code [1-9]|exit status [1-9]|✗|\bFAIL\b|\[error\])").unwrap()
    })
}

fn ansi_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\x1b\[[0-9;]*[a-zA-Z]").unwrap())
}

fn gh_log_line_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^([^\t]*)\t([^\t]*)\t\d{4}-\d{2}-\d{2}T[\d:.]+Z (.*)$").unwrap())
}

fn raw_log_prefix_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^\d{4}-\d{2}-\d{2}T[\d:.]+Z\s*").unwrap())
}

fn job_section_header_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?m)^=== job: (.+?) \(job_id=(\d+)\) ===\n").unwrap())
}

pub struct DistillOptions<'a> {
    pub focus: &'a str,
    pub jobs: &'a [RunJob],
}

struct JobLogChunk {
    job_name: String,
    job_id: u64,
    text: String,
}

pub async fn fetch_failed_logs(
    exec: &GhExec,
    repo: &str,
    run_id: u64,
    job_id: u64,
) -> Result<(String, Vec<RunJob>)> {
    if job_id == 0 {
        let (text, jobs) = fetch_failed_run_logs(exec, repo, run_id).await?;
        return Ok((text, jobs.unwrap_or_default()));
    }

    let run = ci_common::load_run_summary(exec, repo, run_id).await?;
    let job = ci_common::find_run_job(&run.jobs, job_id).ok_or_else(|| {
        CoworkerError::Workflow(format!("job_id {job_id} not found in run {run_id}"))
    })?;
    if !ci_common::is_failed_job_conclusion(&ci_common::job_effective_conclusion(job)) {
        return Err(CoworkerError::Workflow(format!(
            "job_id {job_id} ({}) is not failed",
            job.name
        )));
    }

    let raw = fetch_failed_job_log_text(exec, repo, run_id, job).await?;
    Ok((raw, vec![job.clone()]))
}

pub async fn fetch_failed_run_logs(
    exec: &GhExec,
    repo: &str,
    run_id: u64,
) -> Result<(String, Option<Vec<RunJob>>)> {
    let run_s = run_id.to_string();
    let res = exec
        .run_retry(&["run", "view", &run_s, "-R", repo, "--log-failed"])
        .await;
    if !res.stdout.trim().is_empty() {
        return Ok((res.stdout, None));
    }
    if res.err.is_some() && !ci_common::gh_run_log_recoverable(&res) {
        return Err(res.wrap("failed to fetch failed logs"));
    }

    let (text, jobs) = fetch_failed_job_logs(exec, repo, run_id).await?;
    Ok((text, Some(jobs)))
}

async fn fetch_failed_job_logs(
    exec: &GhExec,
    repo: &str,
    run_id: u64,
) -> Result<(String, Vec<RunJob>)> {
    let run = ci_common::load_run_summary(exec, repo, run_id).await?;
    let (_, _, _, failed_jobs) = ci_common::classify_run_jobs(&run.jobs);
    if failed_jobs.is_empty() {
        return Ok((String::new(), vec![]));
    }

    let mut parts = Vec::new();
    for job in &failed_jobs {
        match fetch_failed_job_log_text(exec, repo, run_id, job).await {
            Ok(raw) if raw.trim().is_empty() => {}
            Ok(raw) => {
                parts.push(format!(
                    "=== job: {} (job_id={}) ===\n{}",
                    job.name,
                    job.database_id,
                    raw.trim()
                ));
            }
            Err(e) => {
                parts.push(format!(
                    "=== job: {} (job_id={}) ===\nfailed to fetch logs: {e}",
                    job.name, job.database_id
                ));
            }
        }
    }
    Ok((parts.join("\n\n"), failed_jobs))
}

pub async fn fetch_failed_job_log_text(
    exec: &GhExec,
    repo: &str,
    run_id: u64,
    job: &RunJob,
) -> Result<String> {
    if ci_common::job_conclusion_skipped(job) {
        return Ok(String::new());
    }

    let job_id_s = job.database_id.to_string();
    let run_id_s = run_id.to_string();
    let attempts: [&[&str]; 4] = [
        &[
            "run",
            "view",
            "-R",
            repo,
            "--job",
            &job_id_s,
            "--log-failed",
        ],
        &[
            "run",
            "view",
            &run_id_s,
            "-R",
            repo,
            "--job",
            &job_id_s,
            "--log-failed",
        ],
        &["run", "view", "-R", repo, "--job", &job_id_s, "--log"],
        &[
            "run", "view", &run_id_s, "-R", repo, "--job", &job_id_s, "--log",
        ],
    ];

    for args in attempts {
        let job_res = exec.run_retry(args).await;
        if !job_res.stdout.trim().is_empty() {
            return Ok(job_res.stdout);
        }
        if job_res.err.is_some() && !ci_common::gh_run_log_recoverable(&job_res) {
            return Err(job_res.wrap(&format!("fetch logs for job {}", job.database_id)));
        }
    }

    if !ci_common::job_logs_ready(job) {
        return Ok(String::new());
    }

    let api_path = format!("repos/{repo}/actions/jobs/{}/logs", job.database_id);
    let res = exec.run_retry(&["api", &api_path]).await;
    if !res.stdout.trim().is_empty() {
        return Ok(res.stdout);
    }
    if res.err.is_some() && !ci_common::gh_run_log_recoverable(&res) {
        return Err(res.wrap(&format!("fetch logs for job {}", job.database_id)));
    }
    Ok(String::new())
}

pub async fn fetch_job_log_text(
    exec: &GhExec,
    repo: &str,
    run_id: u64,
    job: &RunJob,
    prefer_failed: bool,
) -> Result<String> {
    if prefer_failed {
        if let Ok(raw) = fetch_failed_job_log_text(exec, repo, run_id, job).await {
            if !raw.trim().is_empty() {
                return Ok(raw);
            }
        }
    }
    let run_s = run_id.to_string();
    let job_s = job.database_id.to_string();
    let args = ["run", "view", &run_s, "-R", repo, "--job", &job_s, "--log"];
    let job_res = exec.run_retry(&args).await;
    if !job_res.stdout.trim().is_empty() {
        return Ok(job_res.stdout);
    }
    if job_res.err.is_some() && !ci_common::gh_run_log_recoverable(&job_res) {
        return Err(job_res.wrap(&format!("fetch logs for job {}", job.database_id)));
    }
    if !ci_common::job_logs_ready(job) {
        return Ok(String::new());
    }
    let api_path = format!("repos/{repo}/actions/jobs/{}/logs", job.database_id);
    let res = exec.run_retry(&["api", &api_path]).await;
    if !res.stdout.trim().is_empty() {
        return Ok(res.stdout);
    }
    if res.err.is_some() && !ci_common::gh_run_log_recoverable(&res) {
        return Err(res.wrap(&format!("fetch logs for job {}", job.database_id)));
    }
    Ok(String::new())
}

pub fn clean_gh_log(s: &str) -> String {
    clean_gh_log_anchored(s, &[])
}

pub fn clean_gh_log_anchored(s: &str, anchor_steps: &[String]) -> String {
    let s = s.trim_start_matches('\u{feff}');
    let mut out = String::new();
    let mut blank = 0usize;
    for line in s.split('\n') {
        let line = format_gh_log_line_anchored(line, anchor_steps);
        if line.is_empty() {
            if blank > 0 {
                continue;
            }
            blank += 1;
        } else {
            blank = 0;
        }
        out.push_str(&line);
        out.push('\n');
    }
    out.trim().to_string()
}

fn format_gh_log_line_anchored(line: &str, anchor_steps: &[String]) -> String {
    let line = line.trim_end_matches('\r');
    let line = ansi_re().replace_all(line, "");
    if let Some(caps) = gh_log_line_re().captures(&line) {
        let job = caps.get(1).map(|m| m.as_str().trim()).unwrap_or("");
        let step = caps.get(2).map(|m| m.as_str().trim()).unwrap_or("");
        let msg = caps.get(3).map(|m| m.as_str().trim()).unwrap_or("");
        if step.is_empty() || step.eq_ignore_ascii_case("UNKNOWN STEP") {
            let resolved = resolve_anchor_step(msg, anchor_steps);
            if resolved.is_empty() {
                if msg.is_empty() {
                    return format!("{job}:");
                }
                return format!("{job}: {msg}");
            }
            if msg.is_empty() {
                return format!("{job} > {resolved}:");
            }
            return format!("{job} > {resolved}: {msg}");
        }
        if msg.is_empty() {
            return format!("{job} > {step}:");
        }
        return format!("{job} > {step}: {msg}");
    }
    raw_log_prefix_re()
        .replace(&line, "")
        .trim_end()
        .to_string()
}

fn resolve_anchor_step(msg: &str, anchor_steps: &[String]) -> String {
    if anchor_steps.is_empty() {
        return String::new();
    }
    if anchor_steps.len() == 1 {
        return anchor_steps[0].clone();
    }
    let low_msg = msg.to_ascii_lowercase();
    for s in anchor_steps {
        if low_msg.contains(&s.to_ascii_lowercase()) {
            return s.clone();
        }
    }
    anchor_steps[0].clone()
}

pub fn extract_errors(clean: &str) -> (String, usize) {
    extract_errors_with_focus(clean, "last", "")
}

pub fn extract_errors_with_focus(
    clean: &str,
    focus_mode: &str,
    step_name: &str,
) -> (String, usize) {
    let lines: Vec<&str> = clean.split('\n').collect();
    if let Some((body, n)) = extract_marked_lines(&lines, "##[error]") {
        if n > 0 {
            return (apply_error_focus(&body, focus_mode, step_name), n);
        }
    }
    let (body, matches) = extract_regex_lines(&lines);
    if matches == 0 {
        return (String::new(), 0);
    }
    (apply_error_focus(&body, focus_mode, step_name), matches)
}

fn apply_error_focus(body: &str, focus_mode: &str, step_name: &str) -> String {
    let focus_mode = focus_mode.trim().to_ascii_lowercase();
    match focus_mode.as_str() {
        "" | "last" => prefer_last_error_cluster(body),
        "all" => body.to_string(),
        "step" => {
            if step_name.is_empty() {
                prefer_last_error_cluster(body)
            } else {
                filter_error_clusters_by_step(body, step_name)
            }
        }
        _ if focus_mode.starts_with("step:") => {
            filter_error_clusters_by_step(body, focus_mode[5..].trim())
        }
        _ => prefer_last_error_cluster(body),
    }
}

fn filter_error_clusters_by_step(body: &str, step_name: &str) -> String {
    let step_name = step_name.trim();
    if step_name.is_empty() {
        return prefer_last_error_cluster(body);
    }
    let low_step = step_name.to_ascii_lowercase();
    let matched: Vec<&str> = body
        .split("…\n")
        .map(str::trim)
        .filter(|p| !p.is_empty() && p.to_ascii_lowercase().contains(&low_step))
        .collect();
    if matched.is_empty() {
        return prefer_last_error_cluster(body);
    }
    if matched.len() == 1 {
        return matched[0].to_string();
    }
    matched.join("\n…\n")
}

fn extract_marked_lines(lines: &[&str], marker: &str) -> Option<(String, usize)> {
    let mut keep = vec![false; lines.len()];
    let mut matches = 0usize;
    for (i, ln) in lines.iter().enumerate() {
        if !ln.contains(marker) {
            continue;
        }
        matches += 1;
        let lo = i.saturating_sub(ERR_CONTEXT);
        let hi = (i + ERR_CONTEXT).min(lines.len().saturating_sub(1));
        for item in keep.iter_mut().take(hi + 1).skip(lo) {
            *item = true;
        }
        expand_error_block(lines, &mut keep, i);
    }
    if matches == 0 {
        return None;
    }
    Some((join_kept_lines(lines, &keep), matches))
}

fn expand_error_block(lines: &[&str], keep: &mut [bool], start: usize) {
    let mut limit = start + ERR_BLOCK_MAX_LINES;
    if limit >= lines.len() {
        limit = lines.len().saturating_sub(1);
    }
    for j in (start + 1)..=limit {
        if lines[j].trim().is_empty() && j > start + 2 {
            break;
        }
        keep[j] = true;
    }
}

fn extract_regex_lines(lines: &[&str]) -> (String, usize) {
    let re = err_line_re();
    let mut keep = vec![false; lines.len()];
    let mut matches = 0usize;
    for (i, ln) in lines.iter().enumerate() {
        if is_noise_error_line(ln) {
            continue;
        }
        if !re.is_match(ln) {
            continue;
        }
        matches += 1;
        let lo = i.saturating_sub(ERR_CONTEXT);
        let hi = (i + ERR_CONTEXT).min(lines.len().saturating_sub(1));
        for item in keep.iter_mut().take(hi + 1).skip(lo) {
            *item = true;
        }
    }
    (join_kept_lines(lines, &keep), matches)
}

fn join_kept_lines(lines: &[&str], keep: &[bool]) -> String {
    let mut out = String::new();
    let mut gap_open = false;
    let mut last = "";
    for (i, ln) in lines.iter().enumerate() {
        if !keep.get(i).copied().unwrap_or(false) {
            if gap_open {
                out.push_str("…\n");
                gap_open = false;
            }
            continue;
        }
        gap_open = true;
        if is_noise_error_line(ln) && !ln.contains("##[error]") {
            continue;
        }
        if *ln == last {
            continue;
        }
        out.push_str(ln);
        out.push('\n');
        last = ln;
    }
    out.trim().to_string()
}

fn prefer_last_error_cluster(body: &str) -> String {
    let parts: Vec<&str> = body.split("…\n").collect();
    if let Some(last) = parts.last().map(|s| s.trim()).filter(|s| !s.is_empty()) {
        return last.to_string();
    }
    body.to_string()
}

fn is_noise_error_line(ln: &str) -> bool {
    let low = ln.to_ascii_lowercase();
    if ln.contains("##[warning]") {
        return true;
    }
    if low.contains("unable to reserve cache") {
        return true;
    }
    if low.contains("failed to save:") && low.contains("cache") {
        return true;
    }
    if low.contains("npm warn") {
        return true;
    }
    if low.contains("warning:") && !ln.contains("##[error]") {
        return true;
    }
    if low.contains("retrying") || low.contains("attempt 2 of") || low.contains("attempt 3 of") {
        return true;
    }
    if low.contains("downloading") && (low.contains("mb/") || low.contains("mb ")) {
        return true;
    }
    low.contains("uploaded artifact")
}

pub fn distill_failed_log_text(raw_log: &str, opts: DistillOptions<'_>) -> (String, &'static str) {
    let (mut focus_mode, step_name) = parse_log_focus(opts.focus);
    if focus_mode == "step" && step_name.is_empty() {
        focus_mode = "last";
    }

    let chunks = split_logs_into_job_chunks(raw_log);
    if chunks.len() <= 1 {
        let anchor = anchor_steps_for_chunk(
            &JobLogChunk {
                job_name: String::new(),
                job_id: 0,
                text: raw_log.to_string(),
            },
            opts.jobs,
        );
        let clean = clean_gh_log_anchored(raw_log, &anchor);
        return distill_single_log(&clean, focus_mode, &step_name);
    }

    let mut parts = Vec::new();
    let mut overall_mode = "error lines";
    for chunk in chunks {
        let anchor = anchor_steps_for_chunk(&chunk, opts.jobs);
        let clean = clean_gh_log_anchored(&chunk.text, &anchor);
        let (part, part_mode) = distill_single_log(&clean, focus_mode, &step_name);
        if part.trim().is_empty() {
            continue;
        }
        let label = if chunk.job_id > 0 {
            format!("{} (job_id={})", chunk.job_name, chunk.job_id)
        } else {
            chunk.job_name.clone()
        };
        parts.push(format!("[{label}]\n{part}"));
        if part_mode == "log tail" {
            overall_mode = "log tail";
        }
    }
    if parts.is_empty() {
        return (String::new(), "log tail");
    }
    (parts.join("\n\n"), overall_mode)
}

fn parse_log_focus(raw: &str) -> (&'static str, String) {
    let raw = raw.trim();
    let low = raw.to_ascii_lowercase();
    if raw.is_empty() || low == "last" {
        return ("last", String::new());
    }
    if low == "all" {
        return ("all", String::new());
    }
    if let Some(rest) = low.strip_prefix("step:") {
        return ("step", rest.trim().to_string());
    }
    ("last", String::new())
}

fn distill_single_log(clean: &str, focus_mode: &str, step_name: &str) -> (String, &'static str) {
    let (extracted, n) = extract_errors_with_focus(clean, focus_mode, step_name);
    if n > 0 {
        return (extracted, "error lines");
    }
    if clean.trim().is_empty() {
        return (String::new(), "log tail");
    }
    (ci_common::tail_bytes(clean, FALLBACK_TAIL), "log tail")
}

fn split_logs_into_job_chunks(raw: &str) -> Vec<JobLogChunk> {
    let raw = raw.trim();
    if raw.is_empty() {
        return vec![];
    }
    if raw.contains("=== job:") {
        return split_marked_job_sections(raw);
    }
    split_gh_prefix_job_chunks(raw)
}

fn split_marked_job_sections(raw: &str) -> Vec<JobLogChunk> {
    let re = job_section_header_re();
    let matches: Vec<_> = re.captures_iter(raw).collect();
    if matches.is_empty() {
        return split_gh_prefix_job_chunks(raw);
    }

    let mut chunks = Vec::new();
    let mut meta: Vec<(usize, usize, String, u64)> = Vec::new();
    for m in re.captures_iter(raw) {
        let name = m.get(1).map(|x| x.as_str().to_string()).unwrap_or_default();
        let id = m.get(2).and_then(|x| x.as_str().parse().ok()).unwrap_or(0);
        let start = m.get(0).map(|x| x.end()).unwrap_or(0);
        meta.push((m.get(0).map(|x| x.start()).unwrap_or(0), start, name, id));
    }
    for (i, (_, start, name, id)) in meta.iter().enumerate() {
        let end = meta.get(i + 1).map(|(s, _, _, _)| *s).unwrap_or(raw.len());
        let text = raw[*start..end].trim().to_string();
        if !text.is_empty() {
            chunks.push(JobLogChunk {
                job_name: name.clone(),
                job_id: *id,
                text,
            });
        }
    }
    chunks
}

fn split_gh_prefix_job_chunks(raw: &str) -> Vec<JobLogChunk> {
    use std::collections::HashMap;
    let mut by_job: HashMap<String, String> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    for line in raw.split('\n') {
        if let Some(caps) = gh_log_line_re().captures(line) {
            let job = caps.get(1).map(|m| m.as_str().trim()).unwrap_or("");
            if job.is_empty() {
                continue;
            }
            if !by_job.contains_key(job) {
                order.push(job.to_string());
                by_job.insert(job.to_string(), String::new());
            }
            let entry = by_job.get_mut(job).unwrap();
            entry.push_str(line);
            entry.push('\n');
            continue;
        }
        if let Some(last) = order.last() {
            let entry = by_job.get_mut(last).unwrap();
            entry.push_str(line);
            entry.push('\n');
        }
    }
    if order.len() <= 1 {
        return vec![];
    }
    order
        .into_iter()
        .filter_map(|name| {
            let text = by_job.get(&name)?.trim().to_string();
            if text.is_empty() {
                None
            } else {
                Some(JobLogChunk {
                    job_name: name,
                    job_id: 0,
                    text,
                })
            }
        })
        .collect()
}

fn failed_step_names_for_job(job: &RunJob) -> Vec<String> {
    let mut out = Vec::new();
    for step in &job.steps {
        let mut conc = step.conclusion.trim().to_ascii_lowercase();
        if conc.is_empty() {
            conc = step.status.trim().to_ascii_lowercase();
        }
        if conc != "failure" && conc != "timed_out" && conc != "cancelled" {
            continue;
        }
        let name = step.name.trim();
        if !name.is_empty() {
            out.push(name.to_string());
        }
    }
    out
}

fn anchor_steps_for_chunk(chunk: &JobLogChunk, jobs: &[RunJob]) -> Vec<String> {
    for j in jobs {
        if chunk.job_id > 0 && j.database_id == chunk.job_id {
            return failed_step_names_for_job(j);
        }
        if !chunk.job_name.is_empty() && j.name == chunk.job_name {
            return failed_step_names_for_job(j);
        }
    }
    vec![]
}

pub fn format_failed_logs_response(
    run_id: u64,
    synopsis: &str,
    body: &str,
    mode: &str,
    offset_lines: usize,
    max_lines: usize,
) -> String {
    if max_lines > 0 {
        let (page_body, total, next, has_more) =
            ci_common::paginate_lines(body, offset_lines, max_lines);
        if total == 0 && body.trim().is_empty() {
            return format!("{synopsis}\n\nRun {run_id} — empty {mode} (offset {offset_lines}).");
        }
        let start = offset_lines + 1;
        let mut end = next;
        if end > total {
            end = total;
        }
        let page_num = offset_lines / max_lines + 1;
        let mut total_pages = total.div_ceil(max_lines);
        if total_pages == 0 {
            total_pages = 1;
        }
        let header = format!(
            "PAGE: offset={offset_lines} total_lines={total} has_more={has_more} next_offset_lines={next} page={page_num}/{total_pages}"
        );
        let prefix = if offset_lines > 0 {
            String::new()
        } else {
            format!("{synopsis}\n\n")
        };
        return format!(
            "{prefix}{header}\nRun {run_id} — {mode} lines {start}-{end} of {total}\n\n{page_body}"
        );
    }

    if mode == "error lines" {
        let mut line_count = body.matches('\n').count() + 1;
        if line_count == 0 && !body.is_empty() {
            line_count = 1;
        }
        let hint = if max_lines == 0 {
            "\n(hint: pass max_lines=80 to page through long logs)"
        } else {
            ""
        };
        return format!(
            "{synopsis}\n\nRun {run_id} — {line_count} distilled line(s):{hint}\n\n{}",
            ci_common::tail_bytes(body, ERR_BUDGET)
        );
    }

    let hint = if max_lines == 0 {
        "\n(hint: pass max_lines=80 to page through long logs)"
    } else {
        ""
    };
    format!(
        "{synopsis}\n\nRun {run_id} — no recognizable error lines, log tail:{hint}\n\n{}",
        ci_common::tail_bytes(body, FALLBACK_TAIL)
    )
}

pub fn merge_jobs_for_distill(all_jobs: &[RunJob], failed_jobs: &[RunJob]) -> Vec<RunJob> {
    if !failed_jobs.is_empty() {
        return failed_jobs.to_vec();
    }
    let (_, _, _, fj) = ci_common::classify_run_jobs(all_jobs);
    fj
}

pub fn format_distilled_job_logs(
    run_id: u64,
    job_id: u64,
    job_name: &str,
    log_text: &str,
    offset_lines: usize,
    max_lines: usize,
) -> String {
    let clean = clean_gh_log(log_text);
    let (body, mode) = {
        let (extracted, n) = extract_errors(&clean);
        if n > 0 {
            (extracted, "error lines")
        } else if clean.trim().is_empty() {
            (String::new(), "log tail")
        } else {
            (ci_common::tail_bytes(&clean, FALLBACK_TAIL), "log tail")
        }
    };

    if max_lines > 0 {
        let (page, total, next, has_more) =
            ci_common::paginate_lines(&body, offset_lines, max_lines);
        if total == 0 {
            return format!(
                "Run {run_id} job {job_name} (job_id={job_id}) — empty {mode} (offset {offset_lines})."
            );
        }
        let start = offset_lines + 1;
        let end = next.min(total);
        let page_num = offset_lines / max_lines + 1;
        let total_pages = total.div_ceil(max_lines);
        let header = format!(
            "PAGE: offset={offset_lines} total_lines={total} has_more={has_more} next_offset_lines={next} page={page_num}/{total_pages}"
        );
        return format!(
            "{header}\nRun {run_id} job {job_name} (job_id={job_id}) — {mode} lines {start}-{end} of {total}\n\n{page}"
        );
    }

    if mode == "error lines" {
        let line_count = body.matches('\n').count() + if body.is_empty() { 0 } else { 1 };
        format!(
            "Run {run_id} job {job_name} (job_id={job_id}) — {line_count} error line(s):\n\n{}",
            ci_common::tail_bytes(&body, ERR_BUDGET)
        )
    } else {
        format!(
            "Run {run_id} job {job_name} (job_id={job_id}) — no recognizable error lines, showing tail:\n\n{}",
            ci_common::tail_bytes(&body, FALLBACK_TAIL)
        )
    }
}
