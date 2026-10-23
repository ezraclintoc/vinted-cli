//! `vinted mcp install` — register `vinted mcp serve` with agent harnesses.
//!
//! No generic MCP-installer crate exists for Rust (the tooling that does this
//! — `mcp-manager`, `*-setup` bundles — lives in the npm world), so each of
//! the six supported harnesses gets a small dedicated writer here.
//!
//! Supported targets and their config files:
//!
//! | harness    | user config                                | format |
//! | ---------- | ------------------------------------------ | ------ |
//! | claude     | `~/.claude.json` (Claude Code, user scope) | JSON   |
//! | codex      | `~/.codex/config.toml`                     | TOML   |
//! | goose      | `~/.config/goose/config.yaml`              | YAML   |
//! | freebuff   | `.agents/mcp.json` in the current project  | JSON   |
//! | cursor     | `~/.cursor/mcp.json`                       | JSON   |
//! | windsurf   | `~/.codeium/windsurf/mcp_config.json`      | JSON   |
//!
//! Every editor either finds an existing `mcpServers` / `mcp_servers` /
//! `extensions` map (or creates one) and upserts a single `vinted` entry —
//! existing entries and other servers are preserved. YAML comments are not
//! preserved (library limitation); TOML and JSON edits are.

use std::fs;
use std::path::{Path, PathBuf};

/// The command a harness should spawn for our stdio server.
fn server_spec() -> Result<ServerSpec, String> {
    let exe = std::env::current_exe()
        .map_err(|e| format!("cannot locate this binary: {e}"))?
        .to_string_lossy()
        .into_owned();
    Ok(ServerSpec {
        command: exe,
        args: vec!["mcp".into(), "serve".into()],
    })
}

#[derive(Debug, PartialEq)]
struct ServerSpec {
    command: String,
    args: Vec<String>,
}

#[derive(Debug, PartialEq, Copy, Clone)]
enum Target {
    Claude,
    Codex,
    Goose,
    Freebuff,
    Cursor,
    Windsurf,
}

/// Where a harness's user config lives (testable: takes a resolved `home`).
fn config_path(target: &Target, home: &Path) -> Option<PathBuf> {
    let path = match target {
        Target::Claude => home.join(".claude.json"),
        Target::Codex => home.join(".codex").join("config.toml"),
        Target::Goose => home.join(".config").join("goose").join("config.yaml"),
        Target::Freebuff => std::env::current_dir()
            .ok()?
            .join(".agents")
            .join("mcp.json"),
        Target::Cursor => home.join(".cursor").join("mcp.json"),
        Target::Windsurf => home
            .join(".codeium")
            .join("windsurf")
            .join("mcp_config.json"),
    };
    Some(path)
}

/// Directory artifacts that hint the harness is installed.
fn config_dir(target: &Target, home: &Path) -> PathBuf {
    match target {
        Target::Claude => home.join(".claude"),
        Target::Codex => home.join(".codex"),
        Target::Goose => home.join(".config").join("goose"),
        Target::Freebuff => home.join(".agents"),
        Target::Cursor => home.join(".cursor"),
        Target::Windsurf => home.join(".codeium"),
    }
}

fn on_path(name: &str) -> bool {
    let ext = if cfg!(windows) { ".exe" } else { "" };
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|dir| dir.join(format!("{name}{ext}")).is_file()))
        .unwrap_or(false)
}

/// Heuristic: is this harness present on this machine?
fn detected(target: &Target, home: &Path) -> bool {
    match target {
        Target::Claude => on_path("claude") || config_dir(target, home).exists(),
        Target::Codex => on_path("codex") || config_dir(target, home).exists(),
        Target::Goose => on_path("goose") || config_dir(target, home).exists(),
        Target::Freebuff => config_dir(target, home).exists() || on_path("freebuff"),
        Target::Cursor => on_path("cursor") || config_dir(target, home).exists(),
        Target::Windsurf => config_dir(target, home).exists() || on_path("windsurf"),
    }
}

fn label(target: &Target) -> &'static str {
    match target {
        Target::Claude => "Claude Code",
        Target::Codex => "Codex CLI",
        Target::Goose => "Goose",
        Target::Freebuff => "Freebuff",
        Target::Cursor => "Cursor",
        Target::Windsurf => "Windsurf",
    }
}
/// Write (or plan) the `vinted` entry for one harness.
///
/// Returns `Ok(None)` when the harness is already registered with exactly the
/// settings we would write (`--dry-run` prints "already installed"), and
/// `Ok(Some(problem))` describing the change; writes only happen when
/// `dry_run` is false.
fn upsert(
    target: &Target,
    home: &Path,
    spec: &ServerSpec,
    dry_run: bool,
) -> Result<Option<String>, String> {
    match target {
        Target::Codex => upsert_codex(home, spec, dry_run),
        Target::Goose => upsert_goose(home, spec, dry_run),
        _ => upsert_json(target, home, spec, dry_run),
    }
}

// -- JSON (claude / freebuff / cursor / windsurf): {"mcpServers": {"vinted": …}} --

fn upsert_json(
    target: &Target,
    home: &Path,
    spec: &ServerSpec,
    dry_run: bool,
) -> Result<Option<String>, String> {
    let path = config_path(target, home)
        .ok_or_else(|| "could not resolve the working directory".to_string())?;
    let text = fs::read_to_string(&path).unwrap_or_default();
    let mut root: serde_json::Map<String, serde_json::Value> = if text.trim().is_empty() {
        Default::default()
    } else {
        serde_json::from_str::<serde_json::Value>(&text)
            .map_err(|e| format!("{} is not valid JSON ({e})", path.display()))?
            .as_object()
            .cloned()
            .ok_or_else(|| format!("{} is not a JSON object", path.display()))?
    };

    let servers = root
        .entry("mcpServers")
        .or_insert_with(|| serde_json::Value::Object(Default::default()))
        .as_object_mut()
        .ok_or_else(|| format!("'mcpServers' in {} is not an object", path.display()))?;

    let entry = serde_json::json!({ "command": spec.command, "args": spec.args });
    if servers.get("vinted") == Some(&entry) {
        return Ok(None);
    }
    let existed = servers.contains_key("vinted");
    servers.insert("vinted".into(), entry);

    if dry_run {
        return Ok(Some(format!(
            "would {} entry in {}",
            if existed {
                "replace the vinted"
            } else {
                "add a vinted"
            },
            path.display()
        )));
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("creating {parent:?}: {e}"))?;
    }
    let pretty = serde_json::to_string_pretty(&serde_json::Value::Object(root))
        .map_err(|e| e.to_string())?;
    fs::write(&path, pretty + "\n").map_err(|e| format!("writing {path:?}: {e}"))?;
    Ok(Some(format!(
        "{} vinted entry in {}",
        if existed { "replaced" } else { "added" },
        path.display()
    )))
}

// -- TOML (codex): [mcp_servers.vinted] --

fn upsert_codex(home: &Path, spec: &ServerSpec, dry_run: bool) -> Result<Option<String>, String> {
    let path = config_path(&Target::Codex, home).ok_or("no home dir")?;
    let text = fs::read_to_string(&path).unwrap_or_default();
    let mut doc: toml_edit::DocumentMut = text
        .parse()
        .map_err(|e| format!("{} is not valid TOML ({e})", path.display()))?;

    let existing = doc
        .get("mcp_servers")
        .and_then(|i| i.get("vinted"))
        .and_then(|i| i.as_table_like())
        .map(|t| {
            let cmd = t.get("command").and_then(|v| v.as_str());
            let args: Vec<String> = t
                .get("args")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .map(|x| x.as_str().unwrap_or("").to_string())
                        .collect()
                })
                .unwrap_or_default();
            (cmd == Some(spec.command.as_str()), args == spec.args)
        })
        .unwrap_or((false, false));
    if existing.0 && existing.1 {
        return Ok(None);
    }
    let existed = doc
        .get("mcp_servers")
        .and_then(|i| i.get("vinted"))
        .is_some();

    // When we create the "mcp_servers" table fresh, make it implicit so the
    // file shows `[mcp_servers.vinted]` without an empty `[mcp_servers]` header.
    let created_servers = doc.get("mcp_servers").is_none();
    let servers = doc
        .entry("mcp_servers")
        .or_insert(toml_edit::Item::Table(toml_edit::Table::new()))
        .as_table_mut()
        .ok_or("mcp_servers is not a TOML table")?;
    if created_servers {
        servers.set_implicit(true);
    }
    let server = servers
        .entry("vinted")
        .or_insert(toml_edit::Item::Table(toml_edit::Table::new()))
        .as_table_like_mut()
        .ok_or("mcp_servers.vinted is not a TOML table")?;
    server.insert("command", toml_edit::value(spec.command.clone()));
    let mut args = toml_edit::Array::new();
    for a in &spec.args {
        args.push(a.clone());
    }
    server.insert("args", toml_edit::value(args));

    if dry_run {
        return Ok(Some(format!(
            "would {} [mcp_servers.vinted] in {}",
            if existed { "replace" } else { "add" },
            path.display()
        )));
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("creating {parent:?}: {e}"))?;
    }
    fs::write(&path, doc.to_string()).map_err(|e| format!("writing {path:?}: {e}"))?;
    Ok(Some(format!(
        "{} [mcp_servers.vinted] in {}",
        if existed { "replaced" } else { "added" },
        path.display()
    )))
}

// -- YAML (goose): extensions.vinted --

fn upsert_goose(home: &Path, spec: &ServerSpec, dry_run: bool) -> Result<Option<String>, String> {
    use serde_yaml::{Mapping, Value};
    let path = config_path(&Target::Goose, home).ok_or("no home dir")?;
    let text = fs::read_to_string(&path).unwrap_or_default();

    let mut root: Mapping = if text.trim().is_empty() {
        Mapping::new()
    } else {
        serde_yaml::from_str(&text)
            .map_err(|e| format!("{} is not valid YAML ({e})", path.display()))?
    };

    // Idempotency check on the pristine root, BEFORE mutating anything below.
    let already = root
        .get(Value::String("extensions".into()))
        .and_then(|v| v.as_mapping())
        .and_then(|e| e.get(Value::String("vinted".into())))
        .and_then(|v| v.as_mapping())
        .map(|v| {
            let cmd = v
                .get(Value::String("command".into()))
                .and_then(|c| c.as_str());
            let args: Vec<String> = v
                .get(Value::String("args".into()))
                .and_then(|a| a.as_sequence())
                .map(|s| {
                    s.iter()
                        .map(|x| x.as_str().unwrap_or("").to_string())
                        .collect()
                })
                .unwrap_or_default();
            cmd == Some(spec.command.as_str()) && args == spec.args
        })
        .unwrap_or(false);
    let existed = root
        .get(Value::String("extensions".into()))
        .and_then(|v| v.as_mapping())
        .map(|e| e.contains_key(Value::String("vinted".into())))
        .unwrap_or(false);
    if already {
        return Ok(None);
    }

    if !root.contains_key(Value::String("extensions".into())) {
        root.insert(
            Value::String("extensions".into()),
            Value::Mapping(Mapping::new()),
        );
    }
    let extensions = root
        .get_mut(Value::String("extensions".into()))
        .and_then(|v| v.as_mapping_mut())
        .ok_or("goose config.yaml 'extensions' is not a mapping")?;
    if !extensions.contains_key(Value::String("vinted".into())) {
        extensions.insert(
            Value::String("vinted".into()),
            Value::Mapping(Mapping::new()),
        );
    }
    let vinted = extensions
        .get_mut(Value::String("vinted".into()))
        .and_then(|v| v.as_mapping_mut())
        .ok_or("goose extensions 'vinted' is not a mapping")?;
    vinted.insert(Value::String("enabled".into()), Value::Bool(true));
    vinted.insert(Value::String("name".into()), Value::String("vinted".into()));
    vinted.insert(Value::String("type".into()), Value::String("stdio".into()));
    vinted.insert(
        Value::String("command".into()),
        Value::String(spec.command.clone()),
    );
    vinted.insert(
        Value::String("args".into()),
        Value::Sequence(spec.args.iter().map(|a| Value::String(a.clone())).collect()),
    );

    if dry_run {
        return Ok(Some(format!(
            "would {} extensions.vinted in {}",
            if existed { "replace" } else { "add" },
            path.display()
        )));
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("creating {parent:?}: {e}"))?;
    }
    let body = serde_yaml::to_string(&Value::Mapping(root)).map_err(|e| e.to_string())?;
    fs::write(&path, body).map_err(|e| format!("writing {path:?}: {e}"))?;
    Ok(Some(format!(
        "{} extensions.vinted in {} (note: YAML comments get rewritten)",
        if existed { "replaced" } else { "added" },
        path.display()
    )))
}

// ---------------------------------------------------------------------------

fn home_dir() -> Result<PathBuf, String> {
    if let Some(h) = std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
    {
        return Ok(h);
    }
    if let Some(h) = std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
    {
        return Ok(h);
    }
    Err("could not find the home directory".into())
}

/// Selected targets from `--client` values and `--all`.
fn parse_clients(
    clients: &[crate::McpClientArg],
    all: bool,
    home: &Path,
) -> Result<Vec<Target>, String> {
    if all {
        return Ok(vec![
            Target::Claude,
            Target::Codex,
            Target::Goose,
            Target::Freebuff,
            Target::Cursor,
            Target::Windsurf,
        ]);
    }
    let explicit = clients.iter().map(|c| match c {
        crate::McpClientArg::Claude => Target::Claude,
        crate::McpClientArg::Codex => Target::Codex,
        crate::McpClientArg::Goose => Target::Goose,
        crate::McpClientArg::Freebuff => Target::Freebuff,
        crate::McpClientArg::Cursor => Target::Cursor,
        crate::McpClientArg::Windsurf => Target::Windsurf,
    });
    let chosen: Vec<Target> = explicit.collect();
    if !chosen.is_empty() {
        return Ok(chosen);
    }
    // No --client: register with every harness that looks installed.
    let detected: Vec<Target> = ALL_TARGETS
        .iter()
        .filter(|t| detected(t, home))
        .copied()
        .collect();
    if detected.is_empty() {
        return Err(
            "no agent harness config detected on this machine — pass --client (claude, codex, goose, freebuff, cursor, windsurf) or --all"
                .into(),
        );
    }
    Ok(detected)
}

const ALL_TARGETS: [Target; 6] = [
    Target::Claude,
    Target::Codex,
    Target::Goose,
    Target::Freebuff,
    Target::Cursor,
    Target::Windsurf,
];

/// Entry point for `vinted mcp install`.
pub(crate) fn install(
    clients: &[crate::McpClientArg],
    all: bool,
    dry_run: bool,
) -> Result<String, String> {
    let home = home_dir()?;
    let targets = parse_clients(clients, all, &home)?;
    let spec = server_spec()?;

    let mut lines: Vec<String> = Vec::new();
    for target in &targets {
        let change = upsert(target, &home, &spec, dry_run)?;
        lines.push(report(target, &change, dry_run));
    }
    if dry_run {
        lines.insert(0, "dry run (nothing written):".to_string());
    } else {
        lines.insert(0, format!("registered `{} mcp serve` with:", spec.command));
    }
    Ok(lines.join("\n"))
}

fn report(target: &Target, change: &Option<String>, dry_run: bool) -> String {
    let note = match change {
        None => "already installed (no changes)".to_string(),
        Some(msg) => msg.clone(),
    };
    let _ = dry_run; // wording comes from the upserts themselves
    format!("  {:<12} — {note}", label(target))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> ServerSpec {
        ServerSpec {
            command: "/usr/local/bin/vinted".into(),
            args: vec!["mcp".into(), "serve".into()],
        }
    }

    fn temp_home(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("vinted-mcp-test-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn read(home: &Path, rel: &str) -> String {
        fs::read_to_string(home.join(rel)).expect("file should exist")
    }

    #[test]
    fn json_entry_added_then_idempotent() {
        let home = temp_home("json");
        let spec = spec();
        assert!(
            upsert_json(&Target::Cursor, &home, &spec, false)
                .unwrap()
                .is_some()
        );
        let body = read(&home, ".cursor/mcp.json");
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(
            v["mcpServers"]["vinted"]["command"],
            "/usr/local/bin/vinted"
        );
        // Second run: nothing to do.
        assert!(
            upsert_json(&Target::Cursor, &home, &spec, false)
                .unwrap()
                .is_none()
        );
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn json_existing_servers_preserved() {
        let home = temp_home("preserve");
        fs::write(
            home.join(".claude.json"),
            r#"{"mcpServers":{"other":{"command":"x"}},"num":1}"#,
        )
        .unwrap();
        assert!(
            upsert_json(&Target::Claude, &home, &spec(), false)
                .unwrap()
                .is_some()
        );
        let v: serde_json::Value = serde_json::from_str(&read(&home, ".claude.json")).unwrap();
        assert_eq!(v["num"], 1);
        assert_eq!(v["mcpServers"]["other"]["command"], "x");
        assert!(v["mcpServers"]["vinted"].is_object());
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn codex_toml_added_without_losing_config() {
        let home = temp_home("codex");
        fs::create_dir_all(home.join(".codex")).unwrap();
        fs::write(
            home.join(".codex/config.toml"),
            "model = \"gpt-5.2\"\n\n# my config\n[mcp_servers.git]\ncommand = \"git-mcp\"\n",
        )
        .unwrap();
        assert!(upsert_codex(&home, &spec(), false).unwrap().is_some());
        let body = read(&home, ".codex/config.toml");
        assert!(body.contains("model = \"gpt-5.2\""), "existing keys kept");
        assert!(body.contains("# my config"), "comments kept");
        assert!(body.contains("[mcp_servers.git]"));
        assert!(body.contains("[mcp_servers.vinted]"));
        assert!(body.contains("/usr/local/bin/vinted"));
        assert!(body.contains("\"mcp\","));
        assert!(upsert_codex(&home, &spec(), false).unwrap().is_none());
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn goose_yaml_added_and_idempotent() {
        let home = temp_home("goose");
        assert!(upsert_goose(&home, &spec(), false).unwrap().is_some());
        let body = read(&home, ".config/goose/config.yaml");
        assert!(body.contains("extensions:"));
        assert!(body.contains("vinted:"));
        assert!(body.contains("type: stdio"));
        assert!(body.contains("command: /usr/local/bin/vinted"));
        assert!(upsert_goose(&home, &spec(), false).unwrap().is_none());
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn dry_run_writes_nothing() {
        let home = temp_home("dry");
        let msg = upsert_json(&Target::Claude, &home, &spec(), true)
            .unwrap()
            .unwrap();
        assert!(msg.contains("would"));
        assert!(!home.join(".claude.json").exists());
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn detection_uses_config_dirs() {
        // Cannot assert *absence* on the host: a harness may also be detected
        // via its binary on $PATH. Only positive detection is host-independent.
        let home = temp_home("detect");
        fs::create_dir_all(home.join(".codex")).unwrap();
        assert!(detected(&Target::Codex, &home));
        fs::create_dir_all(home.join(".claude")).unwrap();
        assert!(detected(&Target::Claude, &home));
        fs::create_dir_all(home.join(".config/goose")).unwrap();
        assert!(detected(&Target::Goose, &home));
        let _ = fs::remove_dir_all(&home);
    }
}
