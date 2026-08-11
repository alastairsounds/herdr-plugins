#[path = "../src/starship.rs"]
mod starship;
#[path = "../src/fitter.rs"]
mod fitter;

use std::path::Path;
use std::process::Command;

/// `starship prompt` renders the whole configured `format` line in one call,
/// the same mechanism a real terminal prompt uses, instead of one module at a time.
fn invoke_prompt(repo: &Path, config: &Path) -> String {
    let output = Command::new("starship")
        .arg("prompt")
        .arg("--path")
        .arg(repo)
        .arg("--terminal-width")
        .arg("200")
        .env("STARSHIP_CONFIG", config)
        // Starship wraps escapes in zsh's `%{...%}` non-printing markers for PS1 embedding, not plain ANSI.
        .env_remove("STARSHIP_SHELL")
        .output()
        .expect("failed to run starship prompt");
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn main() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
    let config = repo.join("examples/starship-herdr.toml");
    let budget: usize = std::env::args().nth(1).and_then(|a| a.parse().ok()).unwrap_or(26);

    let mut rendered: Vec<fitter::Module> =
        ["directory", "git_branch", "git_status", "git_state", "rust"]
        .into_iter()
        .filter_map(|name| match starship::invoke_module(name, repo, Some(&config)) {
            Ok(content) => Some(fitter::Module { name: name.to_string(), content }),
            Err(e) => {
                eprintln!("{name}: adapter error: {e:?}");
                None
            }
        })
        .collect();
    rendered.push(fitter::Module { name: "starship".to_string(), content: invoke_prompt(repo, &config) });

    println!("--- raw `starship module <name>` output (plus one composite `starship prompt`) ---");
    for m in &rendered {
        println!("{:>12}: {:?}  (rendered: {}\x1b[0m)", m.name, m.content, m.content);
    }

    let fitted = fitter::fit(rendered, budget);

    println!("\n--- fitted to budget={budget} columns ---");
    for m in &fitted {
        println!("{:>12}: {:?}  (rendered: {}\x1b[0m)", m.name, m.content, m.content);
    }

    if std::env::args().any(|a| a == "--push") {
        let workspace_id = std::env::var("HERDR_WORKSPACE_ID")
            .expect("--push requires HERDR_WORKSPACE_ID (run inside a herdr session)");
        // Strip ANSI escapes from the fitted content before pushing to Herdr.
        let stripped: Vec<(String, String)> =
            fitted.iter().map(|m| (m.name.clone(), fitter::strip_ansi(&m.content))).collect();
        let tokens: Vec<(&str, &str)> =
            stripped.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();

        match starship::report_metadata(&workspace_id, &tokens) {
            Ok(()) => println!("\npushed to workspace {workspace_id}"),
            Err(e) => eprintln!("\nreport_metadata error: {e:?}"),
        }
    }
}
