use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::vault_cli::Probe;

pub const ON_PATH: [&str; 2] = ["mindin", "notes-vault-mcp"];

// A vault command setup can offer: where it was found, the words kebbie would run, and what it answered.
#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    pub source: String,
    pub command: Vec<String>,
    pub answer: Option<Probe>,
}

impl Found {
    pub fn shown(&self) -> String {
        self.command.join(" ")
    }

    pub fn answered(&self) -> String {
        match &self.answer {
            Some(probe) if !probe.name.is_empty() => format!("{} {}", probe.name, probe.version),
            Some(probe) => format!("a vault {}", probe.version),
            None => "did not answer".to_string(),
        }
    }
}

pub struct Places<'a> {
    pub claude_dir: &'a Path,
    pub codex_home: Option<&'a Path>,
    pub cursor_dir: &'a Path,
    pub path: &'a [PathBuf],
}

fn read_json(path: &Path) -> Option<Value> {
    std::fs::read_to_string(path).ok().and_then(|raw| serde_json::from_str(&raw).ok())
}

fn words(server: &Value) -> Option<Vec<String>> {
    let command = server["command"].as_str().filter(|command| !command.trim().is_empty())?;
    Some(std::iter::once(command.to_string()).chain(server["args"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string)).collect())
}

fn servers(config: &Value) -> Vec<(String, Vec<String>)> {
    config.as_object().into_iter().flatten().filter_map(|(name, server)| Some((name.clone(), words(server)?))).collect()
}

// Each MCP server in Codex's config.toml, Cursor's mcp.json, and the vault commands on PATH; any of them may run the vault, whatever its name.
fn asked(places: &Places) -> Vec<(String, Vec<String>)> {
    let codex = places.codex_home.map(|home| home.join("config.toml")).and_then(|file| std::fs::read_to_string(file).ok()).and_then(|text| text.parse::<toml::Table>().ok())
        .and_then(|config| serde_json::to_value(config.get("mcp_servers")?).ok()).unwrap_or_default();
    let cursor = read_json(&places.cursor_dir.join("mcp.json")).map(|config| config["mcpServers"].clone()).unwrap_or_default();
    let mut asked: Vec<(String, Vec<String>)> = servers(&codex).into_iter().map(|(name, command)| (format!("Codex's [mcp_servers.{name}]"), command)).collect();
    asked.extend(servers(&cursor).into_iter().map(|(name, command)| (format!("Cursor's mcp.json server {name}"), command)));
    asked.extend(ON_PATH.into_iter().filter(|name| crate::doctor::which(name, places.path).is_some()).map(|name| (format!("{name} on PATH"), vec![name.to_string()])));
    asked
}

// The Claude plugin's server is the command Claude Code starts, so it is offered whatever it answers; every other one only when it answers as a vault.
pub fn candidates(places: &Places, probe: impl Fn(&[String]) -> Option<Probe> + Sync) -> Vec<Found> {
    let plugin = crate::vault_cli::installed(places.claude_dir).and_then(|(id, _, root)| Some((format!("the Claude Code vault plugin {id}"), crate::vault_cli::server_command(&root)?)));
    let asked: Vec<(bool, String, Vec<String>)> = plugin.into_iter().map(|(source, command)| (true, source, command)).chain(asked(places).into_iter().map(|(source, command)| (false, source, command))).collect();
    let answers = crate::sh::in_parallel(&asked, 4, |(_, _, command)| probe(command));
    let mut found: Vec<Found> = Vec::new();
    for ((trusted, source, command), answer) in asked.into_iter().zip(answers) {
        let unique = !found.iter().any(|seen| seen.command == command);
        let runnable = !command.iter().any(|word| word.chars().any(char::is_whitespace));
        if unique && runnable && (trusted || answer.is_some()) {
            found.push(Found { source, command, answer });
        }
    }
    found
}

// Runs the command as setup found it, its program looked up on the given PATH; one that does not answer status with a version is no vault.
pub fn probe_on(path: &[PathBuf], command: &[String]) -> Option<Probe> {
    let (program, rest) = command.split_first()?;
    let program = if program.contains('/') { PathBuf::from(program) } else { crate::doctor::which(program, path)? };
    let words = std::iter::once(program.to_string_lossy().to_string()).chain(rest.iter().cloned()).collect();
    crate::vault_cli::Cli::new(words).probe(false).ok().filter(|probe| !probe.version.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vault(version: &str) -> Probe {
        Probe { name: "mindin".to_string(), version: version.to_string(), configured: true, ..Probe::default() }
    }

    #[test]
    fn setup_finds_the_vault_in_the_claude_plugin_codex_cursor_and_on_path_by_what_answers() {
        let dir = crate::sh::test_dir("vault-found");
        let claude = dir.join("claude");
        let root = claude.join("plugins/cache/gronare/vault/0.3.0");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join(".mcp.json"), r#"{"mcpServers":{"vault":{"command":"uvx","args":["mindin==0.14.0"],"env":{"VAULT_PATH":"${user_config.vault_path}"}}}}"#).unwrap();
        std::fs::write(claude.join("plugins/installed_plugins.json"), serde_json::json!({ "plugins": { "vault@gronare": [{ "installPath": root, "version": "0.3.0" }] } }).to_string()).unwrap();
        let codex = dir.join("codex");
        std::fs::create_dir_all(&codex).unwrap();
        std::fs::write(codex.join("config.toml"), "[mcp_servers.notes]\ncommand = \"uv\"\nargs = [\"run\", \"--project\", \"/src/mindin\", \"mindin\"]\n\n[mcp_servers.github]\ncommand = \"gh-mcp\"\n\n[mcp_servers.same]\ncommand = \"uvx\"\nargs = [\"mindin==0.14.0\"]\n").unwrap();
        let cursor = dir.join("cursor");
        std::fs::create_dir_all(&cursor).unwrap();
        std::fs::write(cursor.join("mcp.json"), r#"{"mcpServers":{"brain":{"command":"/opt/vault/bin/mindin"},"remote":{"url":"https://example.com/mcp"}}}"#).unwrap();
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        crate::executable::write(bin.join("notes-vault-mcp"), "#!/bin/sh\n").unwrap();
        let path = [bin];
        let places = Places { claude_dir: &claude, codex_home: Some(&codex), cursor_dir: &cursor, path: &path };
        let asked = std::sync::Mutex::new(Vec::new());
        let found = candidates(&places, |command| {
            asked.lock().unwrap().push(command.join(" "));
            (command[0] != "gh-mcp" && command[0] != "uvx").then(|| vault("0.14.0"))
        });
        let shown: Vec<(String, String)> = found.iter().map(|found| (found.source.clone(), found.shown())).collect();
        assert_eq!(shown, [
            ("the Claude Code vault plugin vault@gronare".to_string(), "uvx mindin==0.14.0".to_string()),
            ("Codex's [mcp_servers.notes]".to_string(), "uv run --project /src/mindin mindin".to_string()),
            ("Cursor's mcp.json server brain".to_string(), "/opt/vault/bin/mindin".to_string()),
            ("notes-vault-mcp on PATH".to_string(), "notes-vault-mcp".to_string()),
        ], "a server that does not answer as a vault is left out, a command found twice is offered once, and the plugin's is offered even unanswered");
        assert!(asked.lock().unwrap().contains(&"gh-mcp".to_string()), "a server is known by its answer, not its name");
        assert_eq!(found[0].answered(), "did not answer");
        assert_eq!(found[1].answered(), "mindin 0.14.0");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_command_whose_program_is_not_on_the_path_is_not_run() {
        assert_eq!(probe_on(&[], &["uvx".to_string(), "mindin".to_string()]), None);
    }
}
