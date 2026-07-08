use coworker_core::error::Result;

use super::args::CatalogCmd;
use super::terminal::{emit_json, table, use_color_stdout};

pub(crate) async fn run_workflows_list(cmd: CatalogCmd) -> Result<()> {
    use coworker_core::engine::WORKFLOWS;

    let CatalogCmd::List { json } = cmd;
    if json {
        let items: Vec<_> = WORKFLOWS
            .iter()
            .map(|wf| {
                serde_json::json!({
                    "id": wf.id,
                    "description": wf.description,
                    "skills": wf.default_skills,
                })
            })
            .collect();
        emit_json(serde_json::json!(items));
    } else {
        let tty = use_color_stdout();
        let mut rows: Vec<Vec<String>> = Vec::new();
        for wf in WORKFLOWS {
            let skills = if wf.default_skills.is_empty() {
                "—".into()
            } else {
                wf.default_skills.join(", ")
            };
            rows.push(vec![wf.id.to_string(), wf.description.to_string(), skills]);
        }
        println!("{}", table(&["id", "description", "skills"], &rows, tty));
    }
    Ok(())
}

pub(crate) async fn run_catalog_list(root: &str, leaf: &str, cmd: CatalogCmd) -> Result<()> {
    use coworker_core::engine::{load_markdown_spec, load_skill_with_base};
    use std::path::Path;

    let CatalogCmd::List { json } = cmd;
    let root_path = Path::new(root);
    if !root_path.is_dir() {
        if json {
            emit_json(serde_json::json!([]));
        } else {
            eprintln!("(no {root}/ directory)");
        }
        return Ok(());
    }
    let mut entries: Vec<_> = std::fs::read_dir(root_path)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .collect();
    entries.sort_by_key(|e| e.file_name());

    let mut json_items: Vec<serde_json::Value> = Vec::new();
    let mut rows: Vec<Vec<String>> = Vec::new();
    for entry in entries {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('_') {
            continue;
        }
        let path = entry.path().join(leaf);
        if !path.is_file() {
            continue;
        }
        match if root == "skills" {
            load_skill_with_base(&path)
        } else {
            load_markdown_spec(&path)
        } {
            Ok(spec) => {
                let title = if spec.name.is_empty() {
                    name.clone()
                } else {
                    spec.name
                };
                let desc = if spec.description.is_empty() {
                    "—".into()
                } else {
                    spec.description
                };
                let skills = spec.skill_refs.join(", ");
                if json {
                    json_items.push(serde_json::json!({
                        "name": title,
                        "path": path.display().to_string(),
                        "description": desc,
                        "skills": spec.skill_refs,
                    }));
                } else {
                    rows.push(vec![
                        title,
                        path.display().to_string(),
                        desc,
                        if skills.is_empty() {
                            "—".into()
                        } else {
                            skills
                        },
                    ]);
                }
            }
            Err(e) => {
                eprintln!("{}: {e}", path.display());
            }
        }
    }
    if json {
        emit_json(serde_json::json!(json_items));
    } else {
        let tty = use_color_stdout();
        println!(
            "{}",
            table(&["name", "path", "description", "skills"], &rows, tty)
        );
    }
    Ok(())
}
