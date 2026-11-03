use std::path::PathBuf;

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Margin, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Padding, Paragraph, Wrap};
use serde_json::{Value, json};

use crate::board::widgets::{FAINT, GREEN, MANTLE, MAUVE, MUTED, PEACH, RED, SUBTEXT, SURFACE, TEAL, TEXT, YELLOW, bold, choice_lines, style};
use crate::config::{self, Source};
use crate::config_edit::{self, Set};
use crate::agent::Agent;
use crate::agent::codex::Installed;
use crate::agent::codex_setup::{self, Placed, Server};
use crate::agent::login::{self, Login};
use crate::doctor::Repair;
use crate::setup::{self, Asking, KeyPlan, Machine, Outcome};
use crate::sh::Result;
use crate::vault_cli::Plugin;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Step {
    Name,
    Agents,
    ClaudePlugin,
    ClaudeLogin,
    ClaudeApprovals,
    Vault,
    ClaudeUpdates,
    CodexPlugin,
    CodexApprovals,
    CodexVault,
    CodexUpdates,
    VaultCommand,
    Keys,
    Backlog,
    Signals,
    Review,
}

const CLAUDE_STEPS: [Step; 5] = [Step::ClaudePlugin, Step::ClaudeLogin, Step::ClaudeApprovals, Step::Vault, Step::ClaudeUpdates];

impl Step {
    pub const ALL: [Step; 16] = [Step::Name, Step::Agents, Step::ClaudePlugin, Step::ClaudeLogin, Step::ClaudeApprovals, Step::Vault, Step::ClaudeUpdates, Step::CodexPlugin, Step::CodexApprovals, Step::CodexVault, Step::CodexUpdates, Step::VaultCommand, Step::Keys, Step::Backlog, Step::Signals, Step::Review];

    fn index(self) -> usize {
        Step::ALL.iter().position(|step| *step == self).unwrap_or(0)
    }

    fn agent(self) -> Option<Agent> {
        match self {
            Step::ClaudePlugin | Step::ClaudeLogin | Step::ClaudeApprovals | Step::Vault | Step::ClaudeUpdates => Some(Agent::Claude),
            Step::CodexPlugin | Step::CodexApprovals | Step::CodexVault | Step::CodexUpdates => Some(Agent::Codex),
            _ => None,
        }
    }

    fn title(self) -> &'static str {
        match self {
            Step::Name => "Machine name",
            Step::Agents => "Agents",
            Step::ClaudePlugin | Step::CodexPlugin => "Plugin",
            Step::ClaudeLogin => "Login",
            Step::ClaudeApprovals | Step::CodexApprovals => "Approvals",
            Step::Vault => "Vault",
            Step::ClaudeUpdates => "Updates",
            Step::CodexVault => "Vault in Codex",
            Step::CodexUpdates => "Codex updates",
            Step::VaultCommand => "Vault for kebbie",
            Step::Keys => "herdr keys",
            Step::Backlog => "Backlog",
            Step::Signals => "Error signals",
            Step::Review => "Review and apply",
        }
    }

    fn help(self) -> &'static str {
        match self {
            Step::Name => "How your phone and your other machines show this one. One word, like mac or devbox.",
            Step::Agents => "Claude Code runs the lead and the reviewer, so it stays on. Turn on Codex to give tasks a codex profile; each agent you choose gets its own steps next.",
            Step::ClaudePlugin => "kebbie's plugin gives the lead, the reviewer and Claude tasks kebbie's hooks and skills.",
            Step::ClaudeLogin => "Claude Code runs the lead, the reviewer and tasks, so it needs your login.",
            Step::ClaudeApprovals => "kebbie writes each Claude task's permissions into the settings it starts the task with, so Claude Code's own files stay as they are.",
            Step::Vault => "Optional. A note for every task and a log line for every push, in a folder of Markdown or an S3 bucket.",
            Step::ClaudeUpdates => "Claude Code updates itself. When a new version lands, kebbie restarts its sessions on it once they are idle.",
            Step::CodexPlugin => "kebbie's hooks for the Codex sessions it starts: each task gets its role and reports back. Only kebbie's own folder in Codex's plugin cache is written.",
            Step::CodexApprovals => "Codex asks before each command that leaves its sandbox, kebbie's own included: report, tell, ask, steps. A rules file lets the ten commands a task may run go through; push, approve and review still ask, and kebbie refuses them to a task anyway.",
            Step::CodexVault => "Your Claude sessions reach the vault through the vault plugin. Codex needs the same server in its config.toml, so a Codex task keeps its own note instead of asking the lead to write it.",
            Step::CodexUpdates => "Codex does not update itself while it runs, and kebbie turns off its update question in the sessions it starts. kebbie checks once a day for a newer Codex and for models that go away.",
            Step::VaultCommand => "The command kebbie itself runs for task notes, log lines and the board's snapshot, whatever agent runs the task. Each agent reaches the vault through its own MCP server; this is kebbie's own line in its config.toml, and no other tool's file changes.",
            Step::Keys => "Three keys added to the end of herdr's config.toml. The rest of the file stays as it is. Updating is U on the board.",
            Step::Backlog => "Where work waits. Pick any combination; the first one is where the lead files new ideas.",
            Step::Signals => "When kebbie itself crashes or hits some internal errors, it sends its maintainers an error signal, at most once a day. A signal holds only kebbie's version and commit, your OS and CPU architecture, the part of kebbie that failed, the kind of failure and a count; never your code, paths or text. Once a day it also sends an install signal, so they can count active installs: a random id made for this install, kebbie's version, your OS and CPU architecture and the update channel.",
            Step::Review => "Nothing is written until you press ⏎ here.",
        }
    }
}

// The whole guide, or one agent's steps as kebbie setup claude and kebbie setup codex open them.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Scope {
    All,
    Agent(Agent),
}

// How a step that writes another tool's file is done: kebbie writes it, the user adds it from what the guide prints, or not at all.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Way {
    Kebbie,
    Yourself,
    Skip,
}

impl Way {
    const ALL: [Way; 3] = [Way::Kebbie, Way::Yourself, Way::Skip];

    fn at(index: usize) -> Way {
        Way::ALL.get(index).copied().unwrap_or(Way::Kebbie)
    }

    fn index(self) -> usize {
        Way::ALL.iter().position(|way| *way == self).unwrap_or(0)
    }
}

pub struct Env<'a> {
    pub machine: &'a Machine,
    pub config: PathBuf,
    pub codex_home: Option<PathBuf>,
    pub kebbie: String,
    pub state: PathBuf,
}

impl<'a> Env<'a> {
    pub fn current(machine: &'a Machine) -> Result<Env<'a>> {
        Ok(Env { machine, config: config::path(), codex_home: crate::agent::codex::home(), kebbie: crate::peers::kebbie_command().unwrap_or_default(), state: crate::lead::state_dir()? })
    }
}

const S3_FIELDS: [(&str, &str); 4] = [("endpoint", "s3_endpoint"), ("bucket", "s3_bucket"), ("key", "s3_access_key"), ("secret", "s3_secret_key")];

#[derive(Clone, Debug, PartialEq)]
pub struct Facts {
    pub claude_bin: String,
    pub gronare: bool,
    pub name: String,
    pub name_set: bool,
    pub claude: Option<String>,
    pub kebbie_plugin: bool,
    pub codex: Option<String>,
    pub codex_bin: Option<PathBuf>,
    pub claude_login: Login,
    pub codex_login: Login,
    pub claude_stale: Option<String>,
    pub codex_plugin: Installed,
    pub keys: KeyPlan,
    pub sources: Vec<Source>,
    pub sources_set: bool,
    pub vault: Plugin,
    pub vault_path: String,
    pub s3: [String; 4],
    pub hub: Option<String>,
    pub signals: bool,
    pub signals_set: bool,
    pub repairs: Vec<Repair>,
    pub codex_home: Option<PathBuf>,
    pub rules: String,
    pub rules_placed: Placed,
    pub codex_config: Option<String>,
    pub vault_server: Option<Server>,
    pub agent_updates: Option<config::AgentUpdates>,
    pub codex_default: Option<String>,
    pub vault_found: Vec<crate::vault_found::Found>,
    pub vault_command: Option<String>,
}

fn version(program: &str) -> Option<String> {
    let said = crate::sh::run(program, &["--version"], None).ok()?;
    said.split_whitespace().find(|word| word.starts_with(|character: char| character.is_ascii_digit())).map(str::to_string).or(Some(String::new()))
}

fn read_json(path: &std::path::Path) -> Value {
    std::fs::read_to_string(path).ok().and_then(|raw| serde_json::from_str(&raw).ok()).unwrap_or_default()
}

impl Facts {
    pub fn read(env: &Env) -> Facts {
        let machine = config::check_machine(&env.config).map(|(machine, _)| machine).unwrap_or_default();
        let codex = crate::agent::codex::availability(&env.machine.path, env.codex_home.as_deref(), &env.kebbie);
        let options = read_json(&env.machine.claude_dir.join("settings.json"))["pluginConfigs"].as_object().into_iter().flatten()
            .find(|(id, _)| id.starts_with("vault@")).map(|(_, config)| config["options"].clone()).unwrap_or_default();
        let option = |name: &str| options[name].as_str().unwrap_or_default().to_string();
        let herdr = crate::sh::run(&env.machine.herdr, &["plugin", "list", "--json", "--plugin", "kebbie"], None).ok();
        let channel = config::channel_in(&env.config).unwrap_or_default();
        let checkout = crate::doctor::checkout_version(crate::doctor::herdr_plugin(herdr.as_deref(), channel).1.as_ref());
        let claude_plugin = crate::doctor::claude_plugin(Some(&read_json(&env.machine.claude_dir.join("plugins/installed_plugins.json"))), &read_json(&env.machine.claude_dir.join("settings.json")), checkout.as_deref(), channel);
        let kebbie_plugin = setup::kebbie_plugin_installed(&env.machine.claude_dir);
        let rules = codex_setup::rules_text(&codex_setup::rule_forms(&env.kebbie, &env.machine.home.join(".local/bin/kebbie")));
        let vault = crate::vault_cli::plugin(&env.machine.claude_dir);
        let vault_server = match &vault { Plugin::Enabled { root, .. } => codex_setup::vault_server(root, &options), _ => None };
        let cursor = env.machine.home.join(".cursor");
        let places = crate::vault_found::Places { claude_dir: &env.machine.claude_dir, codex_home: env.codex_home.as_deref(), cursor_dir: &cursor, path: &env.machine.path };
        let vault_found = crate::vault_found::candidates(&places, |command| crate::vault_found::probe_on(&env.machine.path, command));
        let codex_default = env.codex_home.as_deref().and_then(|home| crate::agent::codex::configured_model(home).or_else(|| codex_setup::default_model(&crate::agent::codex::cached_models(home)?).map(|model| model.slug.clone())));
        let found = crate::doctor::Found {
            claude_settings: &read_json(&env.machine.claude_dir.join("settings.json")),
            keeper_on: machine.keeper.unwrap_or(true),
            keeper_running: crate::keeper::holder(&env.state).is_some(),
            herdr_list: herdr.as_deref(),
        };
        Facts {
            claude_bin: env.machine.claude.clone(),
            gronare: setup::marketplace_known(&env.machine.claude_dir, "gronare"),
            name: machine.machine_name.clone().unwrap_or_else(config::short_hostname),
            name_set: machine.machine_name.is_some(),
            claude: version(&env.machine.claude),
            kebbie_plugin,
            claude_stale: (kebbie_plugin && claude_plugin.level != crate::doctor::Level::Ok).then_some(claude_plugin.fact),
            codex: codex.binary.as_ref().map(|_| codex.version.as_deref().and_then(|text| text.split_whitespace().find(|word| word.starts_with(|character: char| character.is_ascii_digit()))).unwrap_or_default().to_string()),
            codex_bin: codex.binary.clone(),
            claude_login: login::claude(crate::doctor::which(&env.machine.claude, &env.machine.path).as_deref(), login::TIMEOUT),
            codex_login: if codex.plugin == Installed::Missing { Login::Unknown } else { login::codex(codex.binary.as_deref(), login::TIMEOUT) },
            codex_plugin: codex.plugin.clone(),
            keys: setup::key_plan(&env.machine.herdr_config),
            sources: machine.sources.clone().unwrap_or_else(|| vec![Source::Local]),
            sources_set: machine.sources.is_some(),
            vault,
            vault_path: option("vault_path"),
            s3: S3_FIELDS.map(|(_, name)| option(name)),
            hub: crate::hub::identity::load(&env.state).ok().flatten().map(|identity| crate::hub::host(&identity.url).to_string()),
            signals: machine.report.signals(),
            signals_set: machine.report.signals.is_some(),
            repairs: crate::doctor::repairs(&found),
            rules_placed: env.codex_home.as_deref().map_or(Placed::Missing, |home| codex_setup::rules_placed(home, &rules)),
            rules,
            codex_config: env.codex_home.as_deref().and_then(|home| std::fs::read_to_string(home.join("config.toml")).ok()),
            codex_home: env.codex_home.clone(),
            vault_server,
            agent_updates: machine.agent_updates,
            codex_default,
            vault_found,
            vault_command: machine.worktree.vault_command.clone().filter(|command| !command.trim().is_empty()),
        }
    }

    // The choices on the Vault for kebbie step: the command already set when no tool shows it, each one found, then None.
    fn vault_commands(&self) -> Vec<(String, String, Option<String>)> {
        let found = self.vault_found.iter().map(|found| (found.source.clone(), format!("{} · {}", found.shown(), found.answered()), Some(found.shown())));
        let kept = self.vault_command.clone().filter(|command| !self.vault_found.iter().any(|found| found.shown() == *command)).map(|command| ("Keep [vault] command".to_string(), command.clone(), Some(command)));
        kept.into_iter().chain(found).chain([("None".to_string(), "kebbie keeps tasks without the vault".to_string(), None)]).collect()
    }

    fn vault_command_choice(&self) -> usize {
        let choices = self.vault_commands();
        let at = |wanted: Option<&String>| choices.iter().position(|(_, _, command)| command.as_ref() == wanted);
        match &self.vault_command {
            Some(command) => at(Some(command)),
            None => (choices.len() > 1).then_some(0),
        }.unwrap_or(choices.len() - 1)
    }

    fn vault_choice(&self) -> usize {
        match (&self.vault, self.vault_path.is_empty(), self.s3[1].is_empty()) {
            (Plugin::Missing, _, _) => 0,
            (_, false, _) => 1,
            (_, true, false) => 2,
            _ => 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum KeyChoice {
    Add,
    Move,
    Skip,
}

fn key_choices(plan: &KeyPlan) -> Vec<KeyChoice> {
    if plan.blocked.is_some() || plan.missing.is_empty() {
        return Vec::new();
    }
    let taken: Vec<_> = plan.missing.iter().filter(|wanted| wanted.taken.is_some()).collect();
    if !taken.is_empty() && taken.iter().all(|wanted| wanted.free.is_some()) { vec![KeyChoice::Add, KeyChoice::Move, KeyChoice::Skip] } else { vec![KeyChoice::Add, KeyChoice::Skip] }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Answers {
    pub name: String,
    pub codex: bool,
    pub keys: usize,
    pub sources: Vec<(Source, bool)>,
    pub vault: usize,
    pub vault_path: String,
    pub s3: [String; 4],
    pub signals: usize,
    pub claude_plugin: Way,
    pub codex_plugin: Way,
    pub approvals: Way,
    pub codex_vault: Way,
    pub agent_updates: usize,
    pub vault_command: usize,
}

impl Answers {
    fn from(facts: &Facts) -> Answers {
        let mut sources: Vec<(Source, bool)> = facts.sources.iter().map(|source| (*source, true)).collect();
        sources.extend(Source::ALL.into_iter().filter(|source| !facts.sources.contains(source)).map(|source| (source, false)));
        Answers {
            name: facts.name.clone(),
            codex: crate::agent::chosen_with(&facts.codex_plugin).contains(&Agent::Codex),
            keys: 0,
            sources,
            vault: facts.vault_choice(),
            vault_path: facts.vault_path.clone(),
            s3: facts.s3.clone(),
            signals: usize::from(!facts.signals),
            claude_plugin: Way::Kebbie,
            codex_plugin: Way::Kebbie,
            approvals: Way::Kebbie,
            codex_vault: Way::Kebbie,
            agent_updates: config::AgentUpdates::ALL.iter().position(|mode| Some(*mode) == facts.agent_updates).unwrap_or(0),
            vault_command: facts.vault_command_choice(),
        }
    }

    fn chosen_sources(&self) -> Vec<Source> {
        self.sources.iter().filter(|(_, on)| *on).map(|(source, _)| *source).collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Field {
    Name,
    Path,
    S3(usize),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Spot {
    Choice(usize),
    Check(usize),
    Field(Field),
}

// What the user chose to add themselves; the guide prints it in full where it ends.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Own {
    ClaudePlugin,
    CodexPlugin,
    Rules,
    CodexVault,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Act {
    Link,
    ClaudePlugin,
    CodexPlugin(bool),
    Name(String),
    Keys(Box<KeyPlan>, Vec<toml::Table>),
    Sources(Vec<Source>),
    Vault(Vec<Vec<String>>, Value, String),
    Rules(String),
    CodexVault,
    AgentUpdates(config::AgentUpdates),
    VaultCommand(Option<String>),
    Yourself(Own),
    Signals(bool),
    Repair(Repair),
    Nothing,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Planned {
    pub step: &'static str,
    pub verb: &'static str,
    pub detail: String,
    pub act: Act,
}

pub enum Next {
    Stay,
    Quit,
    Apply,
    Connect,
    Board,
}

// The text a self-added item needs: where it goes, exactly what goes there, and what holds until then.
#[derive(Clone, Debug, PartialEq)]
pub struct Added {
    pub step: String,
    pub said: String,
    pub lines: Vec<String>,
    pub until: String,
}

pub struct Guide {
    pub step: Step,
    pub scope: Scope,
    cursor: usize,
    pub answers: Answers,
    facts: Facts,
    visited: [bool; Step::ALL.len()],
    skipped: [bool; Step::ALL.len()],
    fresh_name: bool,
    error: Option<String>,
    pub applied: Option<Vec<(String, Outcome)>>,
    pub done: bool,
    pub added: Vec<Added>,
    pub ending: Vec<String>,
}

fn home(path: &std::path::Path) -> String {
    crate::area_page::home_relative(path)
}

fn expand(path: &str) -> String {
    config::skill_path(path.trim()).map_or_else(|| path.trim().to_string(), |path| path.display().to_string())
}

fn source_line(source: Source) -> (&'static str, &'static str) {
    match source {
        Source::Local => ("Local file", "backlog.md beside your config"),
        Source::Vault => ("Vault", "backlog notes in your notes vault"),
        Source::Linear => ("Linear", "issues from a Linear team, needs an API key"),
    }
}

fn what(wanted: &setup::Wanted) -> String {
    let said = setup::text(&wanted.binding, "description");
    said.strip_prefix("kebbie: ").unwrap_or(said).to_string()
}

fn short(wanted: &setup::Wanted) -> String {
    what(wanted).trim_start_matches("open ").trim_start_matches("go to ").to_string()
}

const UPDATE_CHOICES: [(&str, &str); 3] = [
    ("Ask me after U", "a window offers the update; nothing installs without your yes"),
    ("Update it with U", "U updates Codex too, then restarts its tasks when they are idle"),
    ("Only tell me", "a notice on the board; you update Codex yourself"),
];

fn update_shown(mode: config::AgentUpdates) -> &'static str {
    match mode {
        config::AgentUpdates::Ask => "ask me after U",
        config::AgentUpdates::Auto => "update it with U",
        config::AgentUpdates::Notify => "only tell me",
    }
}

impl Guide {
    #[cfg(test)]
    pub fn new(facts: Facts, at: Step) -> Guide {
        Guide::scoped(facts, Scope::All, at)
    }

    pub fn scoped(facts: Facts, scope: Scope, at: Step) -> Guide {
        let answers = Answers::from(&facts);
        let mut guide = Guide { step: Step::Name, scope, cursor: 0, answers, facts, visited: [false; Step::ALL.len()], skipped: [false; Step::ALL.len()], fresh_name: true, error: None, applied: None, done: false, added: Vec::new(), ending: Vec::new() };
        guide.go(at);
        guide
    }

    fn vault_chosen(&self) -> bool {
        self.answers.vault > 0
    }

    fn codex_shown(&self) -> bool {
        match self.scope {
            Scope::All => self.answers.codex && self.facts.codex.is_some(),
            Scope::Agent(agent) => agent == Agent::Codex,
        }
    }

    // Asked once something runs the vault on this machine, or kebbie already has a line for it.
    fn vault_command_shown(&self) -> bool {
        self.facts.vault_commands().len() > 1
    }

    fn claude_shown(&self) -> bool {
        self.scope != Scope::Agent(Agent::Codex)
    }

    // The steps this guide walks, in order; an agent's steps follow Agents in the whole guide.
    pub fn steps(&self) -> Vec<Step> {
        let codex: Vec<Step> = [Step::CodexPlugin, Step::CodexApprovals].into_iter().chain(self.vault_chosen().then_some(Step::CodexVault)).chain([Step::CodexUpdates]).collect();
        let mut steps = Vec::new();
        if self.scope == Scope::All {
            steps.extend([Step::Name, Step::Agents]);
        }
        if self.claude_shown() {
            steps.extend(CLAUDE_STEPS);
        }
        if self.codex_shown() {
            steps.extend(codex);
        }
        if self.scope == Scope::All {
            steps.extend(self.vault_command_shown().then_some(Step::VaultCommand));
            steps.extend([Step::Keys, Step::Backlog, Step::Signals]);
        }
        steps.push(Step::Review);
        steps
    }

    fn position(&self) -> usize {
        self.steps().iter().position(|step| *step == self.step).unwrap_or(0)
    }

    fn next_step(&self) -> Step {
        let steps = self.steps();
        steps.get(self.position() + 1).copied().unwrap_or(Step::Review)
    }

    fn previous_step(&self) -> Option<Step> {
        self.position().checked_sub(1).and_then(|at| self.steps().get(at).copied())
    }

    fn spots(&self) -> Vec<Spot> {
        match self.step {
            Step::Name => vec![Spot::Field(Field::Name)],
            Step::Agents => vec![Spot::Check(0), Spot::Check(1)],
            Step::Keys => (0..key_choices(&self.facts.keys).len()).map(Spot::Choice).collect(),
            Step::Backlog => (0..self.answers.sources.len()).map(Spot::Check).collect(),
            Step::Vault => match self.answers.vault {
                1 => vec![Spot::Choice(0), Spot::Choice(1), Spot::Field(Field::Path), Spot::Choice(2)],
                2 => [Spot::Choice(0), Spot::Choice(1), Spot::Choice(2)].into_iter().chain((0..4).map(|at| Spot::Field(Field::S3(at)))).collect(),
                _ => (0..3).map(Spot::Choice).collect(),
            },
            Step::Signals => vec![Spot::Choice(0), Spot::Choice(1)],
            Step::ClaudePlugin | Step::CodexPlugin | Step::CodexApprovals | Step::CodexUpdates => (0..3).map(Spot::Choice).collect(),
            Step::CodexVault => (0..2).map(Spot::Choice).collect(),
            Step::VaultCommand => (0..self.facts.vault_commands().len()).map(Spot::Choice).collect(),
            Step::ClaudeLogin | Step::ClaudeApprovals | Step::ClaudeUpdates | Step::Review => Vec::new(),
        }
    }

    fn spot(&self) -> Option<Spot> {
        self.spots().get(self.cursor).copied()
    }

    fn chosen(&self) -> usize {
        match self.step {
            Step::Keys => self.answers.keys,
            Step::Vault => self.answers.vault,
            Step::Signals => self.answers.signals,
            Step::ClaudePlugin => self.answers.claude_plugin.index(),
            Step::CodexPlugin => self.answers.codex_plugin.index(),
            Step::CodexApprovals => self.answers.approvals.index(),
            Step::CodexVault => self.answers.codex_vault.index(),
            Step::CodexUpdates => self.answers.agent_updates,
            Step::VaultCommand => self.answers.vault_command,
            _ => 0,
        }
    }

    fn go(&mut self, step: Step) {
        self.step = step;
        self.error = None;
        self.fresh_name = true;
        self.cursor = self.spots().iter().position(|spot| *spot == Spot::Choice(self.chosen())).unwrap_or(0);
    }

    fn choose(&mut self, at: usize) {
        match self.step {
            Step::Keys => self.answers.keys = at,
            Step::Vault => self.answers.vault = at,
            Step::Signals => self.answers.signals = at,
            Step::ClaudePlugin => self.answers.claude_plugin = Way::at(at),
            Step::CodexPlugin => self.answers.codex_plugin = Way::at(at),
            Step::CodexApprovals => self.answers.approvals = Way::at(at),
            Step::CodexVault => self.answers.codex_vault = Way::at(at),
            Step::CodexUpdates => self.answers.agent_updates = at,
            Step::VaultCommand => self.answers.vault_command = at,
            _ => {}
        }
    }

    fn field(&mut self, field: Field) -> &mut String {
        match field {
            Field::Name => &mut self.answers.name,
            Field::Path => &mut self.answers.vault_path,
            Field::S3(at) => &mut self.answers.s3[at],
        }
    }

    fn type_char(&mut self, field: Field, character: char) {
        if field == Field::Name && self.fresh_name {
            self.answers.name.clear();
        }
        self.fresh_name = false;
        self.field(field).push(character);
    }

    pub fn paste(&mut self, text: &str) {
        if let Some(Spot::Field(field)) = self.spot() {
            self.fresh_name = false;
            self.field(field).push_str(text.trim());
        }
    }

    fn reset(&mut self, step: Step) {
        let today = Answers::from(&self.facts);
        match step {
            Step::Name => self.answers.name = today.name,
            Step::Agents => self.answers.codex = today.codex,
            Step::Keys => self.answers.keys = key_choices(&self.facts.keys).iter().position(|choice| *choice == KeyChoice::Skip).unwrap_or(0),
            Step::Backlog => self.answers.sources = today.sources,
            Step::Vault => (self.answers.vault, self.answers.vault_path, self.answers.s3) = (today.vault, today.vault_path, today.s3),
            Step::Signals => self.answers.signals = today.signals,
            Step::ClaudePlugin => self.answers.claude_plugin = Way::Skip,
            Step::CodexPlugin => self.answers.codex_plugin = Way::Skip,
            Step::CodexApprovals => self.answers.approvals = Way::Skip,
            Step::CodexUpdates => self.answers.agent_updates = today.agent_updates,
            Step::VaultCommand => self.answers.vault_command = today.vault_command,
            Step::CodexVault | Step::ClaudeLogin | Step::ClaudeApprovals | Step::ClaudeUpdates | Step::Review => {}
        }
    }

    // With kebbie's vault chosen, every agent gets it: Vault in Codex has no skip.
    fn skips(&self) -> bool {
        !matches!(self.step, Step::Name | Step::Review | Step::CodexVault)
    }

    fn check(&self) -> std::result::Result<(), String> {
        match self.step {
            Step::Name => config::valid_machine_name(&self.answers.name).map(|_| ()),
            Step::Vault if self.answers.vault == 1 && self.answers.vault_path.trim().is_empty() => Err("Type the vault folder's path, or pick None.".to_string()),
            Step::Vault if self.answers.vault == 2 && matches!(self.facts.vault, Plugin::Enabled { .. }) && self.answers.s3 == self.facts.s3 => Ok(()),
            Step::Vault if self.answers.vault == 2 => match S3_FIELDS.iter().zip(&self.answers.s3).find(|(_, value)| value.trim().is_empty()) {
                Some(((name, _), _)) => Err(format!("Type the bucket's {name}, or pick None.")),
                None => Ok(()),
            },
            _ => Ok(()),
        }
    }

    pub fn key(&mut self, key: KeyEvent) -> Next {
        if crate::keys::ctrl_c(&key) {
            return Next::Quit;
        }
        if self.applied.is_some() {
            return match key.code {
                _ if !self.done => Next::Stay,
                _ if self.scope != Scope::All => Next::Quit,
                KeyCode::Char('h') if self.offers_hub() => Next::Connect,
                _ => Next::Board,
            };
        }
        let spots = self.spots();
        let field = match self.spot() { Some(Spot::Field(field)) => Some(field), _ => None };
        match key.code {
            KeyCode::Esc => return Next::Quit,
            KeyCode::Up | KeyCode::Down if !spots.is_empty() => {
                let count = spots.len();
                self.cursor = if key.code == KeyCode::Up { (self.cursor + count - 1) % count } else { (self.cursor + 1) % count };
                if let Some(Spot::Choice(at)) = self.spot() {
                    self.choose(at);
                }
                self.error = None;
            }
            KeyCode::Char(character) if field.is_some() => self.type_char(field.unwrap_or(Field::Name), character),
            KeyCode::Backspace if field.is_some() => {
                self.fresh_name = false;
                self.field(field.unwrap_or(Field::Name)).pop();
            }
            KeyCode::Char(' ') => match self.spot() {
                Some(Spot::Check(at)) => self.toggle(at),
                Some(Spot::Choice(at)) => self.choose(at),
                _ => {}
            },
            KeyCode::Enter if self.step == Step::Review => return Next::Apply,
            KeyCode::Enter => match self.check() {
                Ok(()) => {
                    self.visited[self.step.index()] = true;
                    self.skipped[self.step.index()] = false;
                    self.go(self.next_step());
                }
                Err(error) => self.error = Some(error),
            },
            KeyCode::Left => {
                if let Some(previous) = self.previous_step() {
                    self.go(previous);
                }
            }
            KeyCode::Char('s') if self.skips() => {
                self.reset(self.step);
                self.skipped[self.step.index()] = true;
                self.visited[self.step.index()] = false;
                self.go(self.next_step());
            }
            _ => {}
        }
        Next::Stay
    }

    fn toggle(&mut self, at: usize) {
        match self.step {
            Step::Agents if at == 1 && self.facts.codex.is_some() => {
                self.answers.codex = !self.answers.codex;
                if self.answers.codex && self.facts.codex_login == Login::Unknown {
                    self.facts.codex_login = login::codex(self.facts.codex_bin.as_deref(), login::TIMEOUT);
                }
            }
            Step::Backlog => {
                if let Some((_, on)) = self.answers.sources.get_mut(at) {
                    *on = !*on;
                }
            }
            _ => {}
        }
    }

    fn codex_vault_placed(&self) -> Placed {
        self.facts.vault_server.as_ref().map_or(Placed::Missing, |server| codex_setup::vault_placed(self.facts.codex_config.as_deref(), server))
    }

    fn codex_home_shown(&self, file: &str) -> String {
        self.facts.codex_home.as_deref().map_or(format!("~/.codex/{file}"), |codex| home(&codex.join(file)))
    }

    fn codex_plugin_shown(&self) -> String {
        self.facts.codex_home.as_deref().map_or("~/.codex/plugins/cache/kebbie-tasks/kebbie/local".to_string(), |codex| home(&crate::agent::codex::plugin_root(codex)))
    }

    fn in_place(&self, step: Step) -> Option<String> {
        let facts = &self.facts;
        match step {
            Step::Name if facts.name_set => Some(format!("machine.name is \"{}\"", facts.name)),
            Step::ClaudePlugin if facts.kebbie_plugin && facts.claude_stale.is_none() => Some("kebbie's plugin is in Claude Code".to_string()),
            Step::ClaudeLogin if facts.claude_login == Login::In => Some("Claude Code is logged in".to_string()),
            Step::ClaudeApprovals => Some("kebbie gives Claude tasks their rules when they start".to_string()),
            Step::ClaudeUpdates => Some("Claude Code updates itself; kebbie restarts its sessions".to_string()),
            Step::CodexPlugin if facts.codex_plugin == Installed::Current => Some(format!("kebbie's plugin at {}", self.codex_plugin_shown())),
            Step::CodexApprovals if facts.rules_placed == Placed::Current => Some(self.codex_home_shown(codex_setup::RULES_FILE)),
            Step::CodexVault if self.codex_vault_placed() == Placed::Current => Some(format!("[mcp_servers.vault] in {}", self.codex_home_shown("config.toml"))),
            Step::CodexUpdates => facts.agent_updates.map(|mode| format!("update.agents is \"{}\"", mode.key())),
            Step::Keys if facts.keys.blocked.is_none() && facts.keys.missing.is_empty() => Some(format!("{} binds kebbie's keys", home(&facts.keys.config))),
            Step::Backlog if facts.sources_set => Some(format!("board.backlog.sources is [{}]", facts.sources.iter().map(|source| format!("{:?}", source.key())).collect::<Vec<_>>().join(", "))),
            Step::Vault if matches!(facts.vault, Plugin::Enabled { .. }) && facts.vault_choice() > 0 => Some(match facts.vault_choice() { 1 => format!("the vault plugin points at {}", facts.vault_path), _ => format!("the vault plugin points at the S3 bucket {}", facts.s3[1]) }),
            Step::Signals if facts.signals_set => Some(format!("report.signals is {}", facts.signals)),
            Step::VaultCommand => facts.vault_command.as_ref().map(|command| format!("vault.command is \"{command}\"")),
            _ => None,
        }
    }

    // The same checks as kebbie doctor's claude plugin and codex plugin lines, so the guide never calls in place what doctor calls out of date.
    fn to_update(&self, step: Step) -> Option<String> {
        let facts = &self.facts;
        match step {
            Step::ClaudePlugin => facts.claude_stale.clone(),
            Step::CodexPlugin => match &facts.codex_plugin {
                Installed::Stale(files) => Some(format!("kebbie's plugin in Codex is out of date ({})", files.join(", "))),
                Installed::Replaced(_) => Some("Codex replaced kebbie's plugin with a copy of its own".to_string()),
                Installed::UnderOldId(_) => Some("kebbie's plugin in Codex is under kebbie@kebbie, which a repo's marketplace replaces".to_string()),
                _ => None,
            },
            Step::CodexApprovals if facts.rules_placed == Placed::Differs => Some(format!("{} is not the file this kebbie writes", self.codex_home_shown(codex_setup::RULES_FILE))),
            Step::CodexVault if self.codex_vault_placed() == Placed::Differs => Some(format!("[mcp_servers.vault] in {} has other values than the vault plugin's", self.codex_home_shown("config.toml"))),
            _ => None,
        }
    }

    fn skipped(&self, step: Step) -> bool {
        self.skipped[step.index()]
    }

    pub fn plan(&self) -> Vec<Planned> {
        let (facts, answers) = (&self.facts, &self.answers);
        let row = |step, verb, detail: String, act| Planned { step, verb, detail, act };
        let mut rows = Vec::new();
        if self.scope == Scope::All {
            rows.push(row("link", "to do", "~/.local/bin/kebbie".to_string(), Act::Link));
            rows.push(match () {
                _ if self.skipped(Step::Name) => row("machine", "skipped", "stays as it is".to_string(), Act::Nothing),
                _ if facts.name_set && answers.name.trim() == facts.name => row("machine", "already", format!("\"{}\"", facts.name), Act::Nothing),
                _ => row("machine", "to do", format!("\"{}\"", answers.name.trim()), Act::Name(answers.name.trim().to_string())),
            });
        }
        if self.claude_shown() {
            let shown = "kebbie's plugin in Claude Code".to_string();
            rows.push(match answers.claude_plugin {
                _ if self.in_place(Step::ClaudePlugin).is_some() => row("plugin", "already", shown, Act::Nothing),
                Way::Skip => row("plugin", "skipped", "kebbie's plugin stays out of Claude Code".to_string(), Act::Nothing),
                Way::Yourself => row("plugin", "yourself", shown, Act::Yourself(Own::ClaudePlugin)),
                Way::Kebbie => row("plugin", "to do", shown, Act::ClaudePlugin),
            });
            rows.push(self.vault_row());
        }
        if self.scope == Scope::All && !self.codex_shown() && facts.codex.is_some() && crate::agent::chosen_with(&facts.codex_plugin).contains(&Agent::Codex) && !self.skipped(Step::Agents) {
            rows.push(row("plugin", "to do", "kebbie's plugin out of Codex".to_string(), Act::CodexPlugin(false)));
        }
        if self.codex_shown() {
            rows.extend(self.codex_rows());
        }
        if self.scope == Scope::All {
            rows.extend(self.machine_rows());
        }
        rows
    }

    fn codex_rows(&self) -> Vec<Planned> {
        let (facts, answers) = (&self.facts, &self.answers);
        let row = |step, verb, detail: String, act| Planned { step, verb, detail, act };
        let plugin = format!("kebbie's plugin at {}", self.codex_plugin_shown());
        let rules = self.codex_home_shown(codex_setup::RULES_FILE);
        let config = self.codex_home_shown("config.toml");
        let mut rows = vec![match answers.codex_plugin {
            _ if self.in_place(Step::CodexPlugin).is_some() => row("plugin", "already", plugin, Act::Nothing),
            Way::Skip => row("plugin", "skipped", "Codex tasks cannot start without kebbie's plugin".to_string(), Act::Nothing),
            Way::Yourself => row("plugin", "yourself", plugin, Act::Yourself(Own::CodexPlugin)),
            Way::Kebbie => row("plugin", "to do", plugin, Act::CodexPlugin(true)),
        }];
        rows.push(match answers.approvals {
            _ if facts.rules_placed == Placed::Current => row("approvals", "already", rules, Act::Nothing),
            Way::Skip => row("approvals", "skipped", "Codex asks before each kebbie command".to_string(), Act::Nothing),
            Way::Yourself => row("approvals", "yourself", rules, Act::Yourself(Own::Rules)),
            Way::Kebbie => row("approvals", "to do", rules, Act::Rules(facts.rules.clone())),
        });
        if self.vault_chosen() {
            let detail = format!("[mcp_servers.vault] in {config}{}", if answers.vault == 2 { ", keys from the environment" } else { "" });
            rows.push(match answers.codex_vault {
                _ if self.codex_vault_placed() == Placed::Current && !self.vault_changes() => row("vault", "already", detail, Act::Nothing),
                Way::Yourself => row("vault", "yourself", detail, Act::Yourself(Own::CodexVault)),
                _ => row("vault", "to do", detail, Act::CodexVault),
            });
        }
        let mode = config::AgentUpdates::ALL.get(answers.agent_updates).copied().unwrap_or_default();
        rows.push(match () {
            _ if self.skipped(Step::CodexUpdates) => row("updates", "skipped", "stays as it is".to_string(), Act::Nothing),
            _ if facts.agent_updates == Some(mode) => row("updates", "already", format!("{} (update.agents = \"{}\")", update_shown(mode), mode.key()), Act::Nothing),
            _ => row("updates", "to do", format!("{} (update.agents = \"{}\" in kebbie's config.toml)", update_shown(mode), mode.key()), Act::AgentUpdates(mode)),
        });
        rows.push(row("model", "skipped", "Codex profiles follow Codex's default".to_string(), Act::Nothing));
        rows
    }

    // A vault the guide is about to point elsewhere gives Codex's server other values too.
    fn vault_changes(&self) -> bool {
        matches!(self.vault_row().act, Act::Vault(..))
    }

    fn machine_rows(&self) -> Vec<Planned> {
        let (facts, answers) = (&self.facts, &self.answers);
        let row = |step, verb, detail: String, act| Planned { step, verb, detail, act };
        let mut rows = Vec::new();
        let choices = key_choices(&facts.keys);
        rows.push(match (choices.get(answers.keys), &facts.keys.blocked) {
            (_, Some(blocked)) => row("keys", "skipped", blocked.clone(), Act::Nothing),
            (None, _) => row("keys", "already", "kebbie's keys are bound in herdr".to_string(), Act::Nothing),
            (Some(KeyChoice::Skip), _) => row("keys", "skipped", "add them by hand from examples/herdr-config.toml".to_string(), Act::Nothing),
            (Some(choice), _) => {
                let bindings: Vec<toml::Table> = facts.keys.missing.iter().filter_map(|wanted| match (&wanted.taken, choice) {
                    (None, _) => Some(wanted.binding.clone()),
                    (Some(_), KeyChoice::Move) => wanted.moved(),
                    _ => None,
                }).collect();
                let shown = bindings.iter().map(|binding| setup::text(binding, "key")).collect::<Vec<_>>().join(", ");
                let unbound = facts.keys.missing.iter().filter(|wanted| wanted.taken.is_some() && *choice == KeyChoice::Add).map(|wanted| format!(", {} unbound", short(wanted)));
                row("keys", "to do", format!("{shown} in herdr{}", unbound.collect::<String>()), Act::Keys(Box::new(facts.keys.clone()), bindings))
            }
        });
        let sources = answers.chosen_sources();
        let listed = sources.iter().map(|source| source.key()).collect::<Vec<_>>().join(", ");
        rows.push(match () {
            _ if self.skipped(Step::Backlog) => row("backlog", "skipped", "stays as it is".to_string(), Act::Nothing),
            _ if sources == facts.sources => row("backlog", "already", listed, Act::Nothing),
            _ => row("backlog", "to do", listed, Act::Sources(sources)),
        });
        if self.vault_command_shown() {
            rows.push(self.vault_command_row());
        }
        let on = answers.signals == 0;
        let said = |on: bool| if on { "on" } else { "off" };
        rows.push(match () {
            _ if self.skipped(Step::Signals) => row("signals", "skipped", format!("stay {}", said(facts.signals)), Act::Nothing),
            _ if on == facts.signals => row("signals", "already", said(on).to_string(), Act::Nothing),
            _ => row("signals", "to do", said(on).to_string(), Act::Signals(on)),
        });
        rows.extend(facts.repairs.iter().map(|repair| {
            let (step, detail) = repair_shown(*repair);
            row(step, "to do", detail.to_string(), Act::Repair(*repair))
        }));
        rows
    }

    fn vault_command_row(&self) -> Planned {
        let row = |verb, detail: String, act| Planned { step: "vault cmd", verb, detail, act };
        let chosen = self.facts.vault_commands().get(self.answers.vault_command).and_then(|(_, _, command)| command.clone());
        match (chosen, &self.facts.vault_command) {
            _ if self.skipped(Step::VaultCommand) => row("skipped", "stays as it is".to_string(), Act::Nothing),
            (Some(command), Some(kept)) if command == *kept => row("already", format!("vault.command = \"{command}\""), Act::Nothing),
            (Some(command), _) => row("to do", format!("vault.command = \"{command}\" in kebbie's config.toml"), Act::VaultCommand(Some(command))),
            (None, Some(_)) => row("to do", "vault.command out of kebbie's config.toml".to_string(), Act::VaultCommand(None)),
            (None, None) => row("skipped", "kebbie keeps tasks without the vault".to_string(), Act::Nothing),
        }
    }

    fn vault_row(&self) -> Planned {
        let (facts, answers) = (&self.facts, &self.answers);
        let row = |verb, detail: String, act| Planned { step: "vault", verb, detail, act };
        if self.skipped(Step::Vault) || answers.vault == 0 {
            return row("skipped", "not set up".to_string(), Act::Nothing);
        }
        let (values, shown) = if answers.vault == 1 {
            let path = expand(&answers.vault_path);
            (json!({ "vault_path": path }), format!("folder {}", home(std::path::Path::new(&path))))
        } else {
            let values: serde_json::Map<String, Value> = S3_FIELDS.iter().zip(&answers.s3).map(|((_, name), value)| (name.to_string(), Value::from(value.trim()))).collect();
            (Value::Object(values), format!("S3 bucket {}", answers.s3[1].trim()))
        };
        let same = if answers.vault == 1 { expand(&answers.vault_path) == expand(&facts.vault_path) } else { answers.s3.iter().map(|value| value.trim()).eq(facts.s3.iter().map(|value| value.trim())) };
        if matches!(facts.vault, Plugin::Enabled { .. }) && same {
            return row("already", shown, Act::Nothing);
        }
        let mut steps = setup::steps(&facts.claude_bin, &facts.vault, facts.gronare);
        steps.pop();
        row("to do", format!("{shown}, through the vault plugin"), Act::Vault(steps, values, shown))
    }

    // What the last step runs on a file of another tool's, in a few words.
    fn runs(&self) -> Vec<String> {
        let config = self.codex_home_shown("config.toml");
        self.plan().iter().filter_map(|row| match row.act {
            Act::Rules(ref text) => Some(format!("writes {}, {} lines", self.codex_home_shown(codex_setup::RULES_FILE), text.lines().count())),
            Act::CodexVault if self.codex_vault_placed() == Placed::Missing => {
                let lines = self.facts.vault_server.as_ref().map_or("the vault server's".to_string(), |server| server.lines().len().to_string());
                Some(format!("adds {lines} lines to the end of {config}; the rest of the file stays as it is"))
            }
            Act::CodexVault => Some(format!("replaces [mcp_servers.vault] in {config}; the rest of the file stays as it is")),
            _ => None,
        }).collect()
    }

    fn offers_hub(&self) -> bool {
        self.scope == Scope::All && self.facts.hub.is_none()
    }
}

fn repair_shown(repair: Repair) -> (&'static str, &'static str) {
    match repair {
        Repair::StatusLine => ("usage", "kebbie's statusLine in Claude Code's settings.json, for the board's usage bar"),
        Repair::KeeperStart => ("keeper", "start it, so routines run without an open board"),
        Repair::KeeperStop => ("keeper", "stop it, as [keeper] enabled = false says"),
        Repair::HerdrPlugin => ("herdr", "enable kebbie's herdr plugin"),
    }
}

pub trait Hooks {
    fn progress(&mut self, rows: &[(String, Outcome)]);
}

fn when_done(outcome: Outcome, detail: impl Into<String>) -> Outcome {
    if outcome.verb == "done" { Outcome::new("done", detail) } else { outcome }
}

fn write_key(env: &Env, key: &str, set: &Set) -> Outcome {
    match config_edit::write(&env.config, true, false, |text| config_edit::set(text, key, set)) {
        Ok(()) => Outcome::new("done", format!("{key} = {} in {}", set.shown(), home(&env.config))),
        Err(error) => Outcome::new("failed", error.lines().next().unwrap_or_default().to_string()),
    }
}

fn remove_key(env: &Env, key: &str) -> Outcome {
    match config_edit::write(&env.config, true, false, |text| config_edit::remove(text, key)) {
        Ok(()) => Outcome::new("done", format!("{key} out of {}", home(&env.config))),
        Err(error) => Outcome::new("failed", error.lines().next().unwrap_or_default().to_string()),
    }
}

fn codex_home<'a>(env: &'a Env) -> Result<&'a std::path::Path> {
    env.codex_home.as_deref().ok_or_else(|| "neither CODEX_HOME nor HOME is set".to_string())
}

fn codex_plugin(env: &Env, on: bool, shown: &str) -> Outcome {
    let done = codex_home(env).and_then(|home| if on { crate::agent::codex::install(home, &env.kebbie).map(|written| !written.is_empty()) } else { crate::agent::codex::uninstall(home) });
    match done {
        Ok(true) => Outcome::new("done", shown),
        Ok(false) => Outcome::new("already", shown),
        Err(error) => Outcome::new("failed", format!("Codex: {error}")),
    }
}

fn rules(env: &Env, text: &str) -> Outcome {
    match codex_home(env).and_then(|codex| codex_setup::write_rules(codex, text).map(|_| home(&codex_setup::rules_path(codex)))) {
        Ok(path) => Outcome::new("done", path),
        Err(error) => Outcome::new("failed", error),
    }
}

// The vault server is read from the vault plugin as it is now, after this apply's own vault step.
fn fresh_server(env: &Env) -> Option<Server> {
    let (_, _, root) = crate::vault_cli::installed(&env.machine.claude_dir)?;
    codex_setup::vault_server(&root, &codex_setup::vault_options(&env.machine.claude_dir))
}

fn codex_vault(env: &Env) -> Outcome {
    let Some(server) = fresh_server(env) else { return Outcome::new("failed", "the vault plugin is not in Claude Code yet, so its server is unknown; run kebbie setup codex once it is") };
    let result = codex_home(env).and_then(|codex| {
        let path = codex.join("config.toml");
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let new = codex_setup::with_vault(&text, &server)?;
        crate::state::write(&path, new)?;
        Ok(path)
    });
    match result {
        Ok(path) => Outcome::new("done", format!("added [mcp_servers.vault] to {}", home(&path))),
        Err(error) => Outcome::new("failed", error),
    }
}

// Exactly what a self-added item needs and where, read as things are once the rest is applied.
pub fn added(env: &Env, planned: &[Planned]) -> Vec<Added> {
    let codex = env.codex_home.clone().unwrap_or_default();
    planned.iter().filter_map(|row| match row.act {
        Act::Yourself(own) => Some((row.step, own)),
        _ => None,
    }).map(|(step, own)| match own {
        Own::Rules => Added {
            step: step.to_string(),
            said: format!("copy this file to {}:", home(&codex_setup::rules_path(&codex))),
            lines: codex_setup::rules_text(&codex_setup::rule_forms(&env.kebbie, &env.machine.home.join(".local/bin/kebbie"))).lines().map(str::to_string).collect(),
            until: "Codex asks before each kebbie command, and kebbie doctor names the file as missing".to_string(),
        },
        Own::CodexVault => Added {
            step: step.to_string(),
            said: format!("add these lines to the end of {}:", home(&codex.join("config.toml"))),
            lines: fresh_server(env).map_or_else(|| vec!["(the vault plugin is not in Claude Code yet; kebbie setup codex shows them once it is)".to_string()], |server| server.lines()),
            until: "the lead keeps Codex tasks' notes, and kebbie doctor fails the Codex vault line".to_string(),
        },
        Own::ClaudePlugin => {
            let current = env.machine.home.join(".local/share/kebbie/current");
            Added {
                step: step.to_string(),
                said: "run:".to_string(),
                lines: vec![format!("{} plugin marketplace add {}", env.machine.claude, current.display()), format!("{} plugin install kebbie@kebbie", env.machine.claude)],
                until: "kebbie doctor names kebbie's Claude plugin as missing".to_string(),
            }
        }
        Own::CodexPlugin => Added {
            step: step.to_string(),
            said: format!("run this, which writes only kebbie's own folder {}:", home(&crate::agent::codex::plugin_root(&codex))),
            lines: vec!["kebbie setup codex --yes".to_string()],
            until: "Codex tasks do not start, and kebbie doctor names the plugin as missing".to_string(),
        },
    }).collect()
}

fn vault(env: &Env, steps: &[Vec<String>], values: &Value, shown: &str) -> Outcome {
    for step in steps {
        if let Err(error) = crate::sh::run(&step[0], &step[1..].iter().map(String::as_str).collect::<Vec<_>>(), None) {
            return Outcome::new("failed", error);
        }
    }
    let claude = &env.machine.claude;
    match crate::sh::run_with_input(claude, &["plugin", "configure", setup::VAULT_PLUGIN, "--values-stdin"], &values.to_string()) {
        Ok(_) => Outcome::new("done", shown),
        Err(_) => Outcome::new("failed", format!("{claude} plugin configure {} did not take the vault's values; run kebbie setup vault again, or set them in Claude Code's /plugin screen", setup::VAULT_PLUGIN)),
    }
}

fn keys(env: &Env, plan: &KeyPlan, bindings: &[toml::Table], planned: &str) -> Outcome {
    let (outcome, wrote) = setup::add_keys(plan, bindings);
    if !wrote {
        return outcome;
    }
    let reloaded = if setup::reload_herdr(env.machine) == setup::Reload::Done { "herdr reloaded" } else { "restart herdr to use them" };
    when_done(outcome, format!("{}, {reloaded}", planned.replacen(" in herdr", " added", 1)))
}

fn repair(env: &Env, repair: Repair) -> Outcome {
    let said = |result: Result<String>, done: &str| match result {
        Ok(_) => Outcome::new("done", done),
        Err(error) => Outcome::new("failed", error),
    };
    match repair {
        Repair::StatusLine => setup::add_status_line(&env.machine.claude_dir.join("settings.json")),
        Repair::KeeperStart => said(crate::keeper::start_in(&env.state, std::path::Path::new(&env.kebbie), true), "started"),
        Repair::KeeperStop => said(crate::keeper::stop_in(&env.state, std::time::Duration::from_secs(10)), "stopped"),
        Repair::HerdrPlugin => said(crate::sh::run(&env.machine.herdr, &["plugin", "enable", "kebbie"], None), "kebbie's plugin enabled"),
    }
}

pub fn apply(planned: &[Planned], env: &Env, hooks: &mut dyn Hooks) -> Vec<(String, Outcome)> {
    let mut done: Vec<(String, Outcome)> = Vec::new();
    for row in planned {
        let outcome = match &row.act {
            Act::Nothing => Outcome::new(row.verb, row.detail.clone()),
            Act::Link => setup::link_binary(&env.machine.home, &env.machine.path),
            Act::ClaudePlugin => setup::claude_plugin(env.machine),
            Act::CodexPlugin(on) => codex_plugin(env, *on, &row.detail),
            Act::Rules(text) => rules(env, text),
            Act::CodexVault => codex_vault(env),
            Act::AgentUpdates(mode) => when_done(write_key(env, "update.agents", &Set::Text(mode.key().to_string())), format!("update.agents = \"{}\"", mode.key())),
            Act::VaultCommand(Some(command)) => when_done(write_key(env, "vault.command", &Set::Text(command.clone())), format!("vault.command = \"{command}\"")),
            Act::VaultCommand(None) => remove_key(env, "vault.command"),
            Act::Yourself(_) => Outcome::new("yourself", row.detail.clone()),
            Act::Name(name) => when_done(write_key(env, "machine.name", &Set::Text(name.clone())), row.detail.clone()),
            Act::Keys(plan, bindings) => keys(env, plan, bindings, &row.detail),
            Act::Sources(sources) => when_done(write_key(env, "board.backlog.sources", &Set::List(sources.iter().map(|source| source.key().to_string()).collect())), row.detail.clone()),
            Act::Vault(steps, values, shown) => vault(env, steps, values, shown),
            Act::Signals(on) => when_done(write_key(env, "report.signals", &Set::Bool(*on)), row.detail.clone()),
            Act::Repair(fix) => repair(env, *fix),
        };
        done.push((row.step.to_string(), outcome));
        hooks.progress(&done);
    }
    done
}

pub fn without_asking(env: &Env, asking: Asking) -> Vec<(String, Outcome)> {
    let facts = Facts::read(env);
    let (mut keys, wrote) = setup::keys_without_asking(&facts.keys, asking);
    keys.detail.push_str(&setup::herdr_reload(env.machine, wrote));
    let codex = crate::agent::codex::refresh(env.codex_home.as_deref(), &env.kebbie).map(|refreshed| match refreshed {
        Ok(files) if files.is_empty() => Outcome::new("already", "kebbie's Codex plugin is up to date"),
        Ok(files) => Outcome::new("done", format!("kebbie's Codex plugin updated: {}", files.join(", "))),
        Err(error) => Outcome::new("failed", format!("Codex: {error}")),
    });
    let rows = [("link", Some(setup::link_binary(&env.machine.home, &env.machine.path))), ("plugin", Some(setup::claude_plugin(env.machine))), ("codex", codex), ("keys", Some(keys)), ("vault cmd", vault_command_unasked(&facts, &env.config))];
    rows.into_iter().filter_map(|(step, outcome)| Some((step.to_string(), outcome?))).collect()
}

// A run that asks nothing names the vault command it found and the line to set, and writes no choice of its own.
fn vault_command_unasked(facts: &Facts, config: &std::path::Path) -> Option<Outcome> {
    if let Some(command) = &facts.vault_command {
        return Some(Outcome::new("already", format!("vault.command = \"{command}\"")));
    }
    let first = facts.vault_found.first()?;
    let more = match facts.vault_found.len() - 1 {
        0 => String::new(),
        others => format!("; kebbie setup in a terminal offers {others} more"),
    };
    Some(Outcome::new("yourself", format!("{} runs the vault: set vault.command = \"{}\" under [vault] in {}{more}", first.source, first.shown(), home(config))))
}

// What the agent's guide would write besides the plugin, for a run that asks nothing.
pub fn offers(env: &Env, agent: Agent) -> Vec<String> {
    let first = if agent == Agent::Codex { Step::CodexPlugin } else { Step::ClaudePlugin };
    let guide = Guide::scoped(Facts::read(env), Scope::Agent(agent), first);
    let rows: Vec<String> = guide.plan().iter().filter(|row| row.verb == "to do" && !matches!(row.step, "link" | "plugin")).map(|row| format!("  {:<10} {}", row.step, row.detail)).collect();
    if rows.is_empty() {
        return rows;
    }
    std::iter::once(format!("kebbie setup {} in a terminal also offers, each on your choice:", agent.key())).chain(rows).collect()
}

const DONE: &str = "Setup done";
const NEXT: &str = "Next steps";
const LEAD: (&str, &str) = ("prefix+space", "tell the lead what you want done");
pub const PHONE: [&str; 3] = ["", "Your board on your phone: kebbie hub connect, or H on the board", "kebbie.dev/hub"];

fn next_step(command: &str, why: &str) -> String {
    format!("  {command:<21}{why}")
}

// kebbie never writes a shell profile: it names the line and the file for the user's shell.
fn path_step(shell: &str) -> String {
    let line = "export PATH=\"$HOME/.local/bin:$PATH\"";
    let (line, file) = match std::path::Path::new(shell).file_name().and_then(|name| name.to_str()) {
        Some("zsh") => (line, "~/.zshrc"),
        Some("bash") if cfg!(target_os = "macos") => (line, "~/.bash_profile"),
        Some("bash") => (line, "~/.bashrc"),
        Some("fish") => ("fish_add_path ~/.local/bin", "~/.config/fish/config.fish"),
        _ => (line, "your shell's profile"),
    };
    next_step("PATH", &format!("add {line} to {file}, then open a new terminal"))
}

pub fn ending(codex: Option<Login>, checks: &[crate::doctor::Check], shell: &str) -> Vec<String> {
    let count = |ok: bool| checks.iter().filter(|check| if ok { check.level == crate::doctor::Level::Ok } else { matches!(check.level, crate::doctor::Level::Warn | crate::doctor::Level::Fail) }).count();
    let open = count(false);
    let left = if open > 0 { format!(", {open} to look at") } else { String::new() };
    let mut lines = vec![String::new(), format!("{DONE} · doctor: {} ok{left}", count(true)), String::new(), NEXT.to_string()];
    if checks.iter().any(|check| check.name == crate::doctor::ON_PATH && check.fact == crate::doctor::NOT_ON_PATH) {
        lines.push(path_step(shell));
    }
    match codex {
        Some(Login::Out) => lines.push(next_step("codex login", "Codex is not logged in, so codex tasks cannot start")),
        Some(Login::Unknown) => lines.push(next_step("codex login", "if you have not yet, so codex tasks can start")),
        _ => {}
    }
    if open > 0 {
        lines.push(next_step("kebbie doctor", "says what is left and how to fix it"));
    }
    lines.push(next_step(LEAD.0, LEAD.1));
    lines
}

fn row_line(step: &str, outcome: &Outcome) -> Line<'static> {
    Line::from(vec![Span::styled(format!("{:<9}", outcome.verb), style(verb_color(outcome.verb))), Span::styled(format!("{step:<11}"), style(MAUVE)), Span::styled(outcome.detail.clone(), style(TEXT))])
}

fn verb_color(verb: &str) -> Color {
    match verb {
        "done" | "already" => GREEN,
        "failed" => RED,
        "to do" => TEXT,
        "yourself" => YELLOW,
        _ => MUTED,
    }
}

impl Guide {
    fn radio(on: bool) -> Span<'static> {
        if on { Span::styled("(•)", style(GREEN)) } else { Span::styled("( )", style(MUTED)) }
    }

    fn check_mark(on: bool) -> Span<'static> {
        if on { Span::styled("[x]", style(GREEN)) } else { Span::styled("[ ]", style(MUTED)) }
    }

    fn option(&self, spot: Spot, mark: Span<'static>, label: String, about: String) -> Line<'static> {
        let chosen = self.spot() == Some(spot);
        let about = if about.is_empty() { String::new() } else { format!("  · {about}") };
        let line = Line::from(vec![Span::styled(if chosen { "▸ " } else { "  " }, style(MAUVE)), mark, Span::raw(" "), Span::styled(label, if chosen { bold(TEXT) } else { style(TEXT) }), Span::styled(about, style(MUTED))]);
        if chosen { line.style(Style::new().bg(SURFACE)) } else { line }
    }

    fn field_line(&self, field: Field, label: &str, value: &str, secret: bool) -> Line<'static> {
        let chosen = self.spot() == Some(Spot::Field(field));
        let shown = if secret { "•".repeat(value.chars().count()) } else { value.to_string() };
        let caret = if chosen { "▏" } else { "" };
        Line::from(vec![Span::styled(if chosen { "    ▸ " } else { "      " }, style(MAUVE)), Span::styled(format!("{label:<9}"), style(SUBTEXT)), Span::styled(format!("{shown}{caret}"), if chosen { bold(TEXT).bg(MANTLE) } else { style(TEXT) })])
    }

    fn body(&self) -> Vec<Line<'static>> {
        let (facts, answers) = (&self.facts, &self.answers);
        let mut lines = Vec::new();
        match self.step {
            Step::Name => {
                lines.push(self.field_line(Field::Name, "name", &answers.name, false));
                if !facts.name_set {
                    lines.push(Line::from(Span::styled("           from this computer's name", style(MUTED))));
                }
            }
            Step::Agents => {
                let claude = facts.claude.as_ref().map_or("not found · curl -fsSL https://claude.ai/install.sh | bash".to_string(), |version| format!("found {version} · {} · runs the lead, the reviewer and tasks · required", facts.claude_login.shown(login::CLAUDE_FIX)));
                lines.push(self.option(Spot::Check(0), Self::check_mark(true), "Claude Code".to_string(), claude));
                let codex = facts.codex.as_ref().map_or("not found · install Codex CLI first, then run kebbie setup codex".to_string(), |version| format!("found {version} · {} · runs tasks that use a codex profile", facts.codex_login.shown(login::CODEX_FIX)));
                lines.push(self.option(Spot::Check(1), Self::check_mark(answers.codex), "Codex".to_string(), codex));
            }
            Step::Keys => {
                let choices = key_choices(&facts.keys);
                let moving = choices.get(answers.keys) == Some(&KeyChoice::Move);
                if let Some(blocked) = &facts.keys.blocked {
                    lines.push(Line::from(Span::styled(blocked.clone(), style(PEACH))));
                }
                for wanted in &facts.keys.missing {
                    let (mark, key, note) = match (&wanted.taken, moving) {
                        (None, _) => (Span::styled("+", style(GREEN)), wanted.key().to_string(), String::new()),
                        (Some(taken), true) => (Span::styled("+", style(GREEN)), wanted.free.clone().unwrap_or_default(), format!(" · moved off {}, which {taken} uses", wanted.key())),
                        (Some(taken), false) => (Span::styled("!", style(PEACH)), wanted.key().to_string(), format!(" · taken by {taken}")),
                    };
                    lines.push(Line::from(vec![Span::raw("  "), mark, Span::styled(format!(" {key:<14}"), style(TEXT)), Span::styled(format!("{}{note}", what(wanted)), style(MUTED))]));
                }
                if !choices.is_empty() {
                    lines.push(Line::default());
                }
                let taken: Vec<&setup::Wanted> = facts.keys.missing.iter().filter(|wanted| wanted.taken.is_some()).collect();
                for (at, choice) in choices.iter().enumerate() {
                    let (label, about) = match choice {
                        KeyChoice::Add if taken.is_empty() => ("Add them".to_string(), format!("to the end of {}", home(&facts.keys.config))),
                        KeyChoice::Add => (format!("Add them, leave {} to {}", taken.iter().map(|wanted| wanted.key()).collect::<Vec<_>>().join(" and "), taken.iter().filter_map(|wanted| wanted.taken.clone()).collect::<Vec<_>>().join(" and ")), "bind the rest yourself later".to_string()),
                        KeyChoice::Move => (format!("Add them, {}", taken.iter().map(|wanted| format!("{} on {}", short(wanted), wanted.free.clone().unwrap_or_default())).collect::<Vec<_>>().join(", ")), "free in your config".to_string()),
                        KeyChoice::Skip => ("Skip".to_string(), "add them by hand".to_string()),
                    };
                    lines.push(self.option(Spot::Choice(at), Self::radio(answers.keys == at), label, about));
                }
            }
            Step::Backlog => {
                let first = answers.sources.iter().position(|(_, on)| *on);
                for (at, (source, on)) in answers.sources.iter().enumerate() {
                    let (label, about) = source_line(*source);
                    let about = if Some(at) == first { format!("{about} · new ideas go here") } else { about.to_string() };
                    lines.push(self.option(Spot::Check(at), Self::check_mark(*on), label.to_string(), about));
                }
            }
            Step::Vault => {
                for (at, (label, about)) in [("None", "kebbie works the same without it"), ("A local folder", ""), ("An S3 bucket", "endpoint, bucket and keys; the secret is masked")].into_iter().enumerate() {
                    lines.push(self.option(Spot::Choice(at), Self::radio(answers.vault == at), label.to_string(), about.to_string()));
                    match (at, answers.vault) {
                        (1, 1) => lines.push(self.field_line(Field::Path, "path", &answers.vault_path, false)),
                        (2, 2) => lines.extend(S3_FIELDS.iter().enumerate().map(|(field, (label, _))| self.field_line(Field::S3(field), label, &answers.s3[field], field == 3))),
                        _ => {}
                    }
                }
                if answers.vault > 0 {
                    let claude = std::path::Path::new(&facts.claude_bin).file_name().map_or(facts.claude_bin.clone(), |name| name.to_string_lossy().to_string());
                    lines.push(Line::default());
                    lines.push(Line::from(Span::styled(format!("runs: {claude} plugin marketplace add {}", setup::VAULT_MARKETPLACE), style(SUBTEXT)).style(Style::new().bg(MANTLE))));
                    lines.push(Line::from(Span::styled(format!("      {claude} plugin install {}", setup::VAULT_PLUGIN), style(SUBTEXT)).style(Style::new().bg(MANTLE))));
                    lines.push(Line::from(vec![Span::styled(format!("      {claude} plugin configure {}", setup::VAULT_PLUGIN), style(SUBTEXT)), Span::styled(format!(" (with the {} above)", if answers.vault == 1 { "path" } else { "bucket" }), style(MUTED))]));
                }
            }
            Step::VaultCommand => {
                if facts.vault_found.is_empty() {
                    lines.push(Line::from(Span::styled("No other vault command answers on this machine.", style(MUTED))));
                    lines.push(Line::default());
                }
                for (at, (label, about, _)) in facts.vault_commands().into_iter().enumerate() {
                    lines.push(self.option(Spot::Choice(at), Self::radio(answers.vault_command == at), label, about));
                }
            }
            Step::Signals => {
                let labels = if facts.signals { [("Keep them on", "the default"), ("Turn them off", "writes signals = false under [report]")] } else { [("Turn them on", "writes signals = true under [report]"), ("Keep them off", "signals = false stays under [report]")] };
                for (at, (label, about)) in labels.into_iter().enumerate() {
                    lines.push(self.option(Spot::Choice(at), Self::radio(answers.signals == at), label.to_string(), about.to_string()));
                }
                lines.push(Line::default());
                lines.push(Line::from(Span::styled("A bug report with text is always shown to you first and sent only on your y.", style(SUBTEXT))));
            }
            Step::ClaudeLogin => {
                lines.push(Line::from(vec![Span::styled(format!("Claude Code {}", facts.claude.clone().unwrap_or_default()), style(TEXT)), Span::styled(format!(" · {}", facts.claude_login.shown(login::CLAUDE_FIX)), style(MUTED))]));
            }
            Step::ClaudeApprovals | Step::ClaudeUpdates => {}
            Step::ClaudePlugin => self.ways(&mut lines, [("Install kebbie's plugin", "through claude plugin install"), ("I'll add it myself", "shows the commands to run"), ("Skip", "kebbie doctor names it as missing")]),
            Step::CodexPlugin => {
                let root = self.codex_plugin_shown();
                self.ways(&mut lines, [("Install kebbie's plugin", root.as_str()), ("I'll add it myself", "shows the command to run"), ("Skip", "Codex tasks cannot start without it")]);
            }
            Step::CodexApprovals => {
                let file = format!("{}, {} lines", self.codex_home_shown(codex_setup::RULES_FILE), facts.rules.lines().count());
                self.ways(&mut lines, [("Add kebbie's rules file", file.as_str()), ("I'll add it myself", "shows the file to copy"), ("Skip", "Codex asks for each command")]);
            }
            Step::CodexVault => {
                let config = format!("[mcp_servers.vault] in {}", self.codex_home_shown("config.toml"));
                self.ways(&mut lines, [("Add the vault server", config.as_str()), ("I'll add it myself", "shows the lines to copy")]);
                if answers.vault == 2 {
                    lines.push(Line::default());
                    lines.push(Line::from(Span::styled(format!("The keys stay out of the file: Codex reads {} from the environment by name (env_vars), and kebbie starts Codex with them set from the vault plugin's settings.", codex_setup::SECRETS.join(" and ")), style(MUTED))));
                }
            }
            Step::CodexUpdates => {
                self.ways(&mut lines, UPDATE_CHOICES);
                if let Some(model) = &facts.codex_default {
                    lines.push(Line::default());
                    lines.push(Line::from(Span::styled(format!("Codex profiles without a model follow Codex's own default, {model} today."), style(MUTED))));
                }
            }
            Step::Review if self.done => lines.extend(self.done_lines()),
            Step::Review => {
                let rows: Vec<(String, Outcome)> = match &self.applied {
                    Some(done) => {
                        let mut rows = done.clone();
                        rows.extend(self.plan().into_iter().skip(done.len()).map(|row| (row.step.to_string(), Outcome::new("to do", row.detail))));
                        rows
                    }
                    None => self.plan().into_iter().map(|row| (row.step.to_string(), Outcome::new(row.verb, row.detail))).collect(),
                };
                for (step, outcome) in rows {
                    lines.push(row_line(&step, &outcome));
                }
                let runs = if self.applied.is_none() { self.runs() } else { Vec::new() };
                if !runs.is_empty() {
                    lines.push(Line::default());
                    lines.extend(runs.into_iter().map(|said| Line::from(vec![Span::styled("runs: ", style(MUTED)), Span::styled(said, style(MUTED))])));
                }
            }
        }
        if let Some(place) = self.in_place(self.step).filter(|_| self.step != Step::Review) {
            if !lines.is_empty() {
                lines.push(Line::default());
            }
            lines.push(Line::from(vec![Span::styled("✓ in place: ", style(GREEN)), Span::styled(place, style(SUBTEXT))]));
        }
        if let Some(stale) = self.to_update(self.step) {
            if !lines.is_empty() {
                lines.push(Line::default());
            }
            lines.push(Line::from(vec![Span::styled("to update: ", style(PEACH)), Span::styled(stale, style(SUBTEXT))]));
        }
        if let Some(error) = &self.error {
            lines.push(Line::default());
            lines.push(Line::from(Span::styled(error.clone(), style(RED))));
        }
        lines
    }

    fn hint(&self) -> String {
        match self.step {
            _ if self.applied.is_some() && !self.done => "applying…".to_string(),
            _ if self.applied.is_some() && self.scope != Scope::All => "any key closes".to_string(),
            _ if self.applied.is_some() => "any key opens your board".to_string(),
            Step::Name => "type to change · ⏎ next · esc quit".to_string(),
            Step::Review => "⏎ apply · ← back · esc quit without writing".to_string(),
            _ if self.spots().is_empty() => "⏎ next · ← back · esc quit".to_string(),
            step => {
                let toggle = if matches!(step, Step::Agents | Step::Backlog) { "space toggle · " } else { "" };
                let typing = if matches!(self.spot(), Some(Spot::Field(_))) { "type to change · " } else { "" };
                let skip = if self.skips() { "s skip · " } else { "" };
                format!("↑ ↓ choose · {toggle}{typing}⏎ next · ← back · {skip}esc quit")
            }
        }
    }

    pub fn draw(&self, frame: &mut Frame) {
        let area = frame.area().inner(Margin { horizontal: 2, vertical: 1 });
        let [title, _, body] = Layout::vertical([Constraint::Length(1), Constraint::Length(1), Constraint::Min(5)]).areas(area);
        let right = match self.scope {
            Scope::All => "run it again any time · --yes takes the defaults".to_string(),
            Scope::Agent(agent) => format!("kebbie setup {} · esc quits without writing", agent.key()),
        };
        let right = right.as_str();
        let steps = self.steps();
        let left = format!("step {} of {}", self.position() + 1, steps.len());
        let gap = (title.width as usize).saturating_sub(14 + left.len() + right.len()).max(2);
        let right = if gap > 2 { right } else { "" };
        frame.render_widget(Paragraph::new(Line::from(vec![Span::styled("kebbie setup", bold(MAUVE)), Span::raw("  "), Span::styled(left, style(MUTED)), Span::raw(" ".repeat(gap)), Span::styled(right, style(MUTED))])), title);
        let main = if body.width >= 72 {
            let [rail, _, main] = Layout::horizontal([Constraint::Length(22), Constraint::Length(2), Constraint::Min(40)]).areas(body);
            let mut lines: Vec<Line> = Vec::new();
            let mut section = None;
            for (at, step) in steps.iter().enumerate() {
                if self.scope == Scope::All && step.agent() != section {
                    section = step.agent();
                    if let Some(agent) = section {
                        lines.push(Line::from(Span::styled(if agent == Agent::Claude { "  Claude Code" } else { "  Codex" }, style(MUTED))));
                    }
                }
                let current = *step == self.step;
                let mark = match () {
                    _ if current => Span::styled("▸", style(MAUVE)),
                    _ if (self.visited[step.index()] || self.in_place(*step).is_some()) && self.to_update(*step).is_none() => Span::styled("✓", style(GREEN)),
                    _ => Span::styled("·", style(MUTED)),
                };
                let line = Line::from(vec![mark, Span::styled(format!(" {} {}", at + 1, step.title()), if current { bold(TEXT) } else { style(SUBTEXT) })]);
                lines.push(if current { line.style(Style::new().bg(SURFACE)) } else { line });
            }
            frame.render_widget(Paragraph::new(lines), rail);
            main
        } else {
            body
        };
        let [boxed, hint] = Layout::vertical([Constraint::Min(4), Constraint::Length(2)]).areas(main);
        let mut lines = if self.done { Vec::new() } else { vec![Line::from(Span::styled(self.step.help(), style(SUBTEXT))), Line::default()] };
        lines.extend(self.body());
        let framed = |title: &str| Block::bordered().border_type(BorderType::Rounded).border_style(style(FAINT)).padding(Padding::horizontal(1)).title(Span::styled(format!(" {title} "), bold(MAUVE)));
        let tall = |block: &Block, lines: &[Line<'static>], width: u16| Paragraph::new(lines.to_vec()).wrap(Wrap { trim: false }).line_count(block.inner(Rect { width, height: 3, ..boxed }).width) as u16 + 2;
        let phone = (!self.ending.is_empty() && self.offers_hub()).then(|| {
            let block = framed("Your board on your phone");
            let width = boxed.width.min(72);
            let lines = self.phone();
            let height = tall(&block, &lines, width);
            (block, lines, width, height)
        });
        let below = phone.as_ref().map_or(0, |(_, _, _, height)| height + 1);
        let block = framed(if self.done { "Done" } else { self.step.title() });
        let shown = Rect { height: tall(&block, &lines, boxed.width).min(boxed.height.saturating_sub(below)), ..boxed };
        frame.render_widget(Paragraph::new(lines).block(block).wrap(Wrap { trim: false }), shown);
        let mut hint_at = hint.y + 1;
        if let Some((block, lines, width, height)) = phone {
            let at = Rect { y: shown.y + shown.height + 1, width, height: height.min(boxed.height.saturating_sub(shown.height + 1)), ..boxed };
            frame.render_widget(Paragraph::new(lines).block(block).wrap(Wrap { trim: false }), at);
            hint_at = hint_at.min(at.y + at.height + 1);
        }
        frame.render_widget(Paragraph::new(Line::from(Span::styled(self.hint(), style(MUTED)))), Rect { y: hint_at, height: 1, ..hint });
    }

    fn ways<const N: usize>(&self, lines: &mut Vec<Line<'static>>, choices: [(&str, &str); N]) {
        let width = choices.iter().map(|(label, _)| label.chars().count() + 3).max().unwrap_or(0).max(34);
        for (at, (label, about)) in choices.into_iter().enumerate() {
            lines.extend(choice_lines(self.spot() == Some(Spot::Choice(at)), &format!(" {label}"), width, about, MUTED, 200));
        }
    }

    fn until(&self) -> Vec<String> {
        if self.added.is_empty() {
            return Vec::new();
        }
        let mut lines: Vec<String> = self.added.iter().map(|added| format!("Until then {}.", added.until)).collect();
        if !self.added.iter().any(|added| added.step == "plugin") {
            lines.push("Tasks keep running either way.".to_string());
        }
        lines
    }

    fn done_lines(&self) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        for (step, outcome) in self.applied.iter().flatten().filter(|(_, outcome)| !matches!(outcome.verb, "already" | "skipped" | "yourself")) {
            lines.push(row_line(step, outcome));
        }
        if !self.added.is_empty() {
            lines.push(Line::default());
            lines.push(Line::from(Span::styled("You chose to add these yourself:", bold(TEXT))));
            for added in &self.added {
                lines.push(Line::from(vec![Span::styled(format!("{:<9}", "yourself"), style(YELLOW)), Span::styled(format!("{:<11}", added.step), style(MAUVE)), Span::styled(added.said.clone(), style(TEXT))]));
                lines.extend(added.lines.first().map(|first| Line::from(Span::styled(format!("{:<11}  {first}", ""), style(TEAL)))));
                if added.lines.len() > 1 {
                    lines.push(Line::from(Span::styled(format!("{:<11}  … {} more lines, also printed when you quit", "", added.lines.len() - 1), style(MUTED))));
                }
            }
            lines.push(Line::default());
            lines.extend(self.until().into_iter().map(|said| Line::from(Span::styled(said, style(MUTED)))));
        }
        for said in &self.ending {
            lines.push(match (said.as_str(), said.get(..23), said.get(23..)) {
                (_, _, _) if said.starts_with(DONE) => Line::from(vec![Span::styled(DONE, style(GREEN)), Span::styled(said[DONE.len()..].to_string(), style(SUBTEXT))]),
                (NEXT, _, _) => Line::from(Span::styled(NEXT, bold(MAUVE))),
                (_, Some(command), Some(why)) => Line::from(vec![Span::styled(command.to_string(), style(TEXT)), Span::styled(why.to_string(), style(SUBTEXT))]),
                _ => Line::from(Span::styled(said.clone(), style(TEXT))),
            });
        }
        lines
    }

    // The self-added items in full, for the terminal once the guide closes.
    fn added_text(&self) -> Vec<String> {
        if self.added.is_empty() {
            return Vec::new();
        }
        let mut lines = vec![String::new(), "You chose to add these yourself:".to_string()];
        for added in &self.added {
            lines.push(setup::line("yourself", &added.step, &added.said));
            lines.extend(added.lines.iter().map(|line| format!("    {line}")));
        }
        lines.extend(self.until());
        lines
    }

    fn phone(&self) -> Vec<Line<'static>> {
        vec![
            Line::from(Span::styled(format!("Follow tasks and answer questions from anywhere, end-to-end encrypted. Free on {}, or run your own.", crate::hub::host(config::DEFAULT_HUB_URL)), style(TEXT))),
            Line::default(),
            Line::from(vec![Span::styled("h", bold(MAUVE)), Span::styled("  connect now", style(TEXT)), Span::styled("     later: kebbie hub connect, or H on the board", style(MUTED))]),
            Line::from(Span::styled(PHONE[2], style(SUBTEXT))),
        ]
    }

    fn text(&self) -> String {
        self.said(self.offers_hub())
    }

    fn said(&self, phone: bool) -> String {
        match &self.applied {
            None => "Nothing written: kebbie setup runs the guide again whenever you want.".to_string(),
            Some(rows) => {
                let mut lines: Vec<String> = rows.iter().map(|(step, outcome)| outcome.line(step)).collect();
                lines.extend(self.added_text());
                lines.extend(self.ending.iter().cloned());
                if phone {
                    lines.extend(PHONE.iter().map(|line| line.to_string()));
                }
                lines.join("\n")
            }
        }
    }
}

struct Screen<'a> {
    terminal: &'a mut ratatui::DefaultTerminal,
    guide: &'a mut Guide,
}

impl Hooks for Screen<'_> {
    fn progress(&mut self, rows: &[(String, Outcome)]) {
        self.guide.applied = Some(rows.to_vec());
        let guide = &*self.guide;
        let _ = self.terminal.draw(|frame| guide.draw(frame));
    }
}

fn ask(env: &Env, scope: Scope, at: Step) -> Result<(Next, Guide)> {
    use ratatui::crossterm::event::{self, Event, KeyEventKind};
    let mut guide = Guide::scoped(Facts::read(env), scope, at);
    let mut terminal = crate::board::terminal::init_terminal();
    let result = (|| -> Result<Next> {
        loop {
            terminal.draw(|frame| guide.draw(frame)).map_err(|error| error.to_string())?;
            match event::read().map_err(|error| error.to_string())? {
                Event::Key(key) if key.kind == KeyEventKind::Press => match guide.key(key) {
                    Next::Stay => {}
                    Next::Apply => {
                        let planned = guide.plan();
                        guide.applied = Some(Vec::new());
                        let rows = apply(&planned, env, &mut Screen { terminal: &mut terminal, guide: &mut guide });
                        guide.added = added(env, &planned);
                        if scope == Scope::All {
                            let codex = (guide.answers.codex && !guide.skipped[Step::Agents.index()]).then(|| login::codex(guide.facts.codex_bin.as_deref(), login::TIMEOUT));
                            guide.ending = ending(codex, &crate::doctor::checks(&crate::doctor::Env::current()), &std::env::var("SHELL").unwrap_or_default());
                        }
                        guide.applied = Some(rows);
                        guide.done = true;
                    }
                    next => return Ok(next),
                },
                Event::Paste(text) => guide.paste(&text),
                _ => {}
            }
        }
    })();
    crate::board::terminal::restore_terminal();
    Ok((result?, guide))
}

pub fn run(env: &Env, scope: Scope, at: Step) -> Result<String> {
    let (next, guide) = ask(env, scope, at)?;
    match next {
        Next::Board if std::env::var("HERDR_PANE_ID").is_ok_and(|pane| !pane.is_empty()) => {
            println!("{}\n", guide.text());
            return crate::lead::show();
        }
        Next::Board => println!("{}\n", guide.text()),
        Next::Connect => {
            println!("{}\n", guide.said(false));
            println!("{}", crate::hub::cli(Some("connect"), &[], None)?);
        }
        _ => return Ok(guide.text()),
    }
    crate::board::run(false)
}

pub fn first_run(claude_dir: &std::path::Path, terminal: bool) -> bool {
    terminal && !setup::kebbie_plugin_installed(claude_dir)
}

// A board opened before kebbie's Claude Code plugin is installed, as after herdr's own install, starts in the guide; quitting it opens the board as it is.
pub fn board(overlay: bool) -> Result<String> {
    let claude_dir = crate::agent::claude::home().unwrap_or_default();
    if first_run(&claude_dir, setup::in_terminal()) && crate::permit::user_only("kebbie setup").is_ok() {
        let machine = setup::machine()?;
        let env = Env::current(&machine)?;
        let (next, guide) = ask(&env, Scope::All, Step::Name)?;
        match next {
            Next::Connect => {
                println!("{}\n", guide.said(false));
                println!("{}", crate::hub::cli(Some("connect"), &[], None)?);
            }
            Next::Quit => {}
            _ => println!("{}\n", guide.text()),
        }
        if !matches!(next, Next::Quit) && std::env::var("HERDR_PANE_ID").is_ok_and(|pane| !pane.is_empty()) {
            let _ = crate::lead::restart(false, false);
        }
    }
    crate::board::run(overlay)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Lab {
        dir: PathBuf,
        machine: Machine,
        log: PathBuf,
    }

    impl Drop for Lab {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    impl Lab {
        fn new(name: &str) -> Lab {
            let dir = crate::sh::test_dir(name);
            let (machine, log) = setup::tests::machine(&dir);
            Lab { dir, machine, log }
        }

        fn env(&self) -> Env<'_> {
            Env { machine: &self.machine, config: self.dir.join("kebbie/config.toml"), codex_home: Some(self.dir.join("codex")), kebbie: "/opt/kebbie".to_string(), state: self.dir.join("state") }
        }

        fn calls(&self) -> String {
            std::fs::read_to_string(&self.log).unwrap_or_default()
        }

        fn config(&self) -> String {
            std::fs::read_to_string(self.dir.join("kebbie/config.toml")).unwrap_or_default()
        }

        fn herdr(&self) -> String {
            std::fs::read_to_string(&self.machine.herdr_config).unwrap_or_default()
        }
    }

    struct Fake {
        drawn: usize,
    }

    impl Hooks for Fake {
        fn progress(&mut self, _: &[(String, Outcome)]) {
            self.drawn += 1;
        }
    }

    fn press(guide: &mut Guide, codes: &[KeyCode]) {
        for code in codes {
            guide.key(KeyEvent::from(*code));
        }
    }

    fn type_text(guide: &mut Guide, text: &str) {
        for character in text.chars() {
            guide.key(KeyEvent::from(KeyCode::Char(character)));
        }
    }

    fn screen(guide: &Guide) -> String {
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(230, 40)).unwrap();
        terminal.draw(|frame| guide.draw(frame)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        (0..buffer.area.height).map(|y| (0..buffer.area.width).map(|x| buffer[(x, y)].symbol().to_string()).collect::<String>().trim_end().to_string()).collect::<Vec<_>>().join("\n")
    }

    fn plan(guide: &Guide) -> Vec<String> {
        guide.plan().iter().map(|row| format!("{} {} {}", row.verb, row.step, row.detail)).collect()
    }

    fn fresh(lab: &Lab) -> Guide {
        let mut facts = Facts::read(&lab.env());
        facts.name = "mac".to_string();
        facts.claude = Some("2.1.3".to_string());
        facts.codex = Some("0.163.0".to_string());
        Guide::new(facts, Step::Name)
    }

    #[test]
    fn the_guide_walks_each_chosen_agents_own_steps_and_writes_nothing_before_apply() {
        let lab = Lab::new("guide-walk");
        let mut guide = fresh(&lab);
        let shown = screen(&guide);
        assert!(shown.contains("kebbie setup  step 1 of 11") && shown.contains("▸ 1 Machine name") && shown.contains("· 11 Review and apply") && !shown.contains("Hub"), "{shown}");
        assert!(shown.contains("  Claude Code") && shown.contains("· 3 Plugin") && shown.contains("· 6 Vault") && !shown.contains("Codex updates"), "Claude Code's steps follow Agents, Codex's only once it is chosen: {shown}");
        assert!(shown.contains("▸ name     mac▏") && shown.contains("from this computer's name") && shown.contains("type to change · ⏎ next · esc quit"), "{shown}");
        type_text(&mut guide, "devbox");
        press(&mut guide, &[KeyCode::Enter]);
        assert_eq!((guide.step, guide.answers.name.as_str()), (Step::Agents, "devbox"), "typing replaces the name it was given");
        let shown = screen(&guide);
        assert!(shown.contains("✓ 1 Machine name") && shown.contains("[x] Claude Code  · found 2.1.3 · logged in · runs the lead, the reviewer and tasks · required") && shown.contains("[ ] Codex  · found 0.163.0"), "{shown}");
        assert!(shown.contains("[ ] Codex  · found 0.163.0 · login not checked · runs tasks"), "no answer from codex is no verdict: {shown}");
        guide.facts.codex_login = Login::Out;
        guide.facts.claude_login = Login::Out;
        let shown = screen(&guide);
        assert!(shown.contains("found 2.1.3 · not logged in: run claude auth login") && shown.contains("found 0.163.0 · not logged in: run codex login"), "{shown}");
        press(&mut guide, &[KeyCode::Char(' ')]);
        assert!(!guide.answers.codex, "space on Claude Code changes nothing: it stays on");
        let codex = lab.dir.join("codex-bin");
        crate::executable::write(&codex, "#!/bin/sh\necho 'Logged in using ChatGPT'\n").unwrap();
        (guide.facts.codex_bin, guide.facts.codex_login) = (Some(codex), Login::Unknown);
        press(&mut guide, &[KeyCode::Down, KeyCode::Char(' ')]);
        assert!(guide.answers.codex && guide.facts.codex_login == Login::In, "Codex's login is checked once you choose it");
        assert!(screen(&guide).contains("  Codex") && screen(&guide).contains("· 10 Codex updates") && screen(&guide).contains("· 14 Review and apply"), "Codex's steps join once it is on: {}", screen(&guide));
        press(&mut guide, &[KeyCode::Enter]);
        assert_eq!(guide.step, Step::ClaudePlugin);
        let shown = screen(&guide);
        assert!(shown.contains("▌ Install kebbie's plugin") && shown.contains("I'll add it myself") && shown.contains("shows the commands to run") && shown.contains("Skip"), "{shown}");
        press(&mut guide, &[KeyCode::Enter]);
        assert_eq!(guide.step, Step::ClaudeLogin);
        assert!(screen(&guide).contains("Claude Code 2.1.3 · not logged in: run claude auth login") && screen(&guide).contains("⏎ next · ← back · esc quit"), "{}", screen(&guide));
        press(&mut guide, &[KeyCode::Enter]);
        assert!(screen(&guide).contains("✓ in place: kebbie gives Claude tasks their rules when they start"), "{}", screen(&guide));
        press(&mut guide, &[KeyCode::Enter, KeyCode::Down]);
        assert_eq!((guide.step, guide.answers.vault), (Step::Vault, 1), "moving onto an option picks it");
        let shown = screen(&guide);
        assert!(shown.contains("(•) A local folder") && shown.contains("path") && shown.contains("plugin install vault@gronare"), "{shown}");
        press(&mut guide, &[KeyCode::Enter]);
        assert!(screen(&guide).contains("Type the vault folder's path, or pick None."), "an empty path stops the step");
        press(&mut guide, &[KeyCode::Down]);
        type_text(&mut guide, "/notes/vault");
        press(&mut guide, &[KeyCode::Enter]);
        assert_eq!(guide.step, Step::ClaudeUpdates);
        assert!(screen(&guide).contains("✓ in place: Claude Code updates itself; kebbie restarts its sessions"));
        press(&mut guide, &[KeyCode::Enter, KeyCode::Enter, KeyCode::Enter]);
        assert_eq!(guide.step, Step::CodexVault, "with kebbie's vault chosen, Codex gets its Vault step");
        assert!(!screen(&guide).contains("s skip") && !screen(&guide).contains("Skip"), "the vault every agent gets has no skip: {}", screen(&guide));
        press(&mut guide, &[KeyCode::Char('s')]);
        assert_eq!(guide.step, Step::CodexVault, "s skips nothing here");
        press(&mut guide, &[KeyCode::Enter, KeyCode::Enter]);
        assert_eq!(guide.step, Step::Keys);
        let shown = screen(&guide);
        assert!(shown.contains("+ prefix+space  go to the lead, or back") && shown.contains("+ prefix+t      open the board") && !shown.contains("prefix+u"), "{shown}");
        assert!(shown.contains("(•) Add them  · to the end of") && shown.contains("( ) Skip"), "{shown}");
        press(&mut guide, &[KeyCode::Enter]);
        assert_eq!(guide.step, Step::Backlog);
        assert!(screen(&guide).contains("[x] Local file  · backlog.md beside your config · new ideas go here"));
        press(&mut guide, &[KeyCode::Down, KeyCode::Char(' '), KeyCode::Up, KeyCode::Char(' '), KeyCode::Enter]);
        assert_eq!(guide.answers.chosen_sources(), [Source::Vault], "the first one checked is where the lead files ideas");
        assert_eq!(guide.step, Step::Signals, "the backlog leads straight to the error signals");
        assert!(screen(&guide).contains("never your code, paths or text") && screen(&guide).contains("A bug report with text is always shown to you first"));
        press(&mut guide, &[KeyCode::Down, KeyCode::Enter]);
        assert_eq!(guide.step, Step::Review);
        let rules = format!("{}/codex/rules/kebbie.rules", lab.dir.display());
        assert_eq!(plan(&guide), [
            "to do link ~/.local/bin/kebbie".to_string(),
            "to do machine \"devbox\"".to_string(),
            "to do plugin kebbie's plugin in Claude Code".to_string(),
            "to do vault folder /notes/vault, through the vault plugin".to_string(),
            format!("to do plugin kebbie's plugin at {}/codex/plugins/cache/kebbie-tasks/kebbie/local", lab.dir.display()),
            format!("to do approvals {rules}"),
            format!("to do vault [mcp_servers.vault] in {}/codex/config.toml", lab.dir.display()),
            "to do updates ask me after U (update.agents = \"ask\" in kebbie's config.toml)".to_string(),
            "skipped model Codex profiles follow Codex's default".to_string(),
            "to do keys prefix+space, prefix+f, prefix+t in herdr".to_string(),
            "to do backlog vault".to_string(),
            "to do signals off".to_string(),
            "to do usage kebbie's statusLine in Claude Code's settings.json, for the board's usage bar".to_string(),
            "to do keeper start it, so routines run without an open board".to_string(),
        ], "the doctor's fixes are rows of the plan like every other");
        let shown = screen(&guide);
        assert!(shown.contains("to do    usage      kebbie's statusLine in Claude Code's settings.json") && shown.contains("⏎ apply · ← back · esc quit without writing"), "{shown}");
        assert!(shown.contains(&format!("runs: writes {rules}, 30 lines")) && shown.contains("runs: adds the vault server's lines to the end of"), "{shown}");
        assert_eq!((lab.config(), lab.herdr(), lab.calls()), (String::new(), String::new(), String::new()), "nothing is written or run before ⏎ on the last step");
        assert!(!lab.dir.join("codex").exists() && !lab.machine.home.join(".local/bin/kebbie").exists() && !lab.machine.claude_dir.join("settings.json").exists());
        assert!(matches!(guide.key(KeyEvent::from(KeyCode::Esc)), Next::Quit));
        assert_eq!(guide.text(), "Nothing written: kebbie setup runs the guide again whenever you want.");
    }

    #[test]
    fn apply_writes_and_runs_each_line_then_a_rerun_starts_from_what_is_in_place() {
        let lab = Lab::new("guide-apply");
        let mut guide = fresh(&lab);
        guide.facts.repairs.retain(|repair| *repair != Repair::KeeperStart);
        press(&mut guide, &[KeyCode::Enter, KeyCode::Down, KeyCode::Char(' '), KeyCode::Enter, KeyCode::Enter, KeyCode::Enter, KeyCode::Enter, KeyCode::Down, KeyCode::Down]);
        type_text(&mut guide, "/notes/vault");
        press(&mut guide, &[KeyCode::Enter, KeyCode::Enter, KeyCode::Enter, KeyCode::Down, KeyCode::Enter, KeyCode::Enter, KeyCode::Enter, KeyCode::Enter, KeyCode::Down, KeyCode::Char(' '), KeyCode::Enter, KeyCode::Down, KeyCode::Enter]);
        assert_eq!(guide.step, Step::Review);
        assert!(matches!(guide.key(KeyEvent::from(KeyCode::Enter)), Next::Apply));
        let mut fake = Fake { drawn: 0 };
        let planned = guide.plan();
        let rows = apply(&planned, &lab.env(), &mut fake);
        let lines: Vec<String> = rows.iter().map(|(step, outcome)| format!("{} {step} {}", outcome.verb, outcome.detail)).collect();
        assert_eq!(lines, [
            "done link ~/.local/bin/kebbie".to_string(),
            "done machine \"mac\"".to_string(),
            format!("done plugin installed kebbie@kebbie in Claude Code from {}", lab.machine.home.join(".local/share/kebbie/current").display()),
            "done vault folder /notes/vault".to_string(),
            format!("done plugin kebbie's plugin at {}/codex/plugins/cache/kebbie-tasks/kebbie/local", lab.dir.display()),
            format!("yourself approvals {}/codex/rules/kebbie.rules", lab.dir.display()),
            "failed vault the vault plugin is not in Claude Code yet, so its server is unknown; run kebbie setup codex once it is".to_string(),
            "done updates update.agents = \"ask\"".to_string(),
            "skipped model Codex profiles follow Codex's default".to_string(),
            "done keys prefix+space, prefix+f, prefix+t added, restart herdr to use them".to_string(),
            "done backlog local, vault".to_string(),
            "done signals off".to_string(),
            "done usage statusLine added to Claude Code's settings.json".to_string(),
        ], "each line says what it did in a few words");
        assert_eq!(fake.drawn, 13, "each line is drawn as it finishes");
        assert_eq!(lab.config(), "[machine]\nname = \"mac\"\n\n[update]\nagents = \"ask\"\n\n[board.backlog]\nsources = [\"local\", \"vault\"]\n\n[report]\nsignals = false\n", "setup never writes [hub]");
        assert_eq!(lab.herdr().matches("[[keys.command]]").count(), 3);
        assert!(lab.dir.join("codex/plugins/cache/kebbie-tasks/kebbie/local/hooks/hooks.json").is_file(), "Codex gets kebbie's plugin");
        assert!(!lab.dir.join("codex/rules").exists(), "a file the user adds is never written");
        let added = added(&lab.env(), &planned);
        assert_eq!((added.len(), added[0].step.as_str(), added[0].lines.len()), (1, "approvals", 30));
        let settings: Value = serde_json::from_str(&std::fs::read_to_string(lab.machine.claude_dir.join("settings.json")).unwrap()).unwrap();
        assert_eq!(settings["statusLine"]["command"], crate::doctor::STATUS_LINE_COMMAND);
        let calls = lab.calls();
        assert!(calls.contains("plugin marketplace add https://github.com/gronare/claude-plugins.git\nplugin install vault@gronare\nplugin configure vault@gronare --values-stdin\n"), "{calls}");
        assert!(std::fs::read_to_string(lab.dir.join("claude.stdin")).unwrap().contains("{\"vault_path\":\"/notes/vault\"}"), "the vault's values go in on stdin");
        assert!(Facts::read(&lab.env()).repairs.iter().all(|repair| *repair != Repair::StatusLine), "a fix applied is no row the next time");
        let plugins = lab.machine.claude_dir.join("plugins");
        let vault_root = lab.dir.join("vault-plugin");
        std::fs::create_dir_all(vault_root.join("hooks")).unwrap();
        std::fs::write(vault_root.join("hooks/hooks.json"), "uvx notes-vault-mcp==0.6.0 hook session-start").unwrap();
        std::fs::write(vault_root.join(".mcp.json"), r#"{"mcpServers":{"vault":{"command":"uvx","args":["notes-vault-mcp==0.6.0"]}}}"#).unwrap();
        std::fs::write(plugins.join("installed_plugins.json"), json!({ "plugins": { "kebbie@kebbie": [{ "version": "0.27.0" }], "vault@gronare": [{ "version": "0.3.0", "installPath": vault_root }] } }).to_string()).unwrap();
        std::fs::write(lab.machine.claude_dir.join("settings.json"), json!({ "pluginConfigs": { "vault@gronare": { "options": { "vault_path": "/notes/vault" } } } }).to_string()).unwrap();
        let rerun = Guide::new(Facts::read(&lab.env()), Step::Name);
        let mut signals = Guide::new(Facts::read(&lab.env()), Step::Signals);
        let shown = screen(&signals);
        assert!(shown.contains("( ) Turn them on") && shown.contains("▸ (•) Keep them off"), "signals that are off offer to turn them on: {shown}");
        press(&mut signals, &[KeyCode::Up, KeyCode::Enter]);
        assert_eq!(plan(&signals).iter().find(|row| row.contains(" signals ")).map(String::as_str), Some("to do signals on"));
        assert_eq!(rerun.answers.name, "mac");
        assert_eq!(rerun.answers.chosen_sources(), [Source::Local, Source::Vault]);
        assert_eq!((rerun.answers.signals, rerun.answers.codex, rerun.answers.vault, rerun.answers.vault_path.as_str()), (1, true, 1, "/notes/vault"));
        let shown = screen(&rerun);
        assert!(shown.contains("✓ in place: machine.name is \"mac\"") && shown.contains("herdr keys") && shown.contains("Error signals"), "{shown}");
        assert!(shown.lines().any(|line| line.contains("✓") && line.contains("herdr keys")) && shown.lines().any(|line| line.contains("✓") && line.contains("Error signals")), "the rail marks what is in place: {shown}");
        let mut rerun = rerun;
        rerun.facts.codex = Some("0.163.0".to_string());
        rerun.go(Step::Review);
        let plan = plan(&rerun);
        for row in ["already machine \"mac\"", "already keys kebbie's keys are bound in herdr", "already backlog local, vault", "already vault folder /notes/vault", "already signals off", "already updates ask me after U (update.agents = \"ask\")"] {
            assert!(plan.iter().any(|line| line == row), "{row} missing from {plan:?}");
        }
    }

    #[test]
    fn a_stale_codex_plugin_shows_as_to_update_and_apply_rewrites_it() {
        let lab = Lab::new("guide-stale-codex");
        let plugins = lab.machine.claude_dir.join("plugins");
        std::fs::create_dir_all(&plugins).unwrap();
        std::fs::write(plugins.join("installed_plugins.json"), json!({ "plugins": { "kebbie@kebbie": [{ "version": "0.27.0" }] } }).to_string()).unwrap();
        crate::agent::codex::install(&lab.dir.join("codex"), "/elsewhere/kebbie").unwrap();
        let mut guide = fresh(&lab);
        assert!(guide.answers.codex, "an installed plugin keeps Codex on");
        guide.go(Step::CodexPlugin);
        let shown = screen(&guide);
        assert!(shown.contains("to update: kebbie's plugin in Codex is out of date (bin/kebbie-hook)") && !shown.contains("✓ in place"), "{shown}");
        assert!(shown.lines().any(|line| line.contains("▸ 8 Plugin")), "{shown}");
        let rows = apply(&guide.plan(), &lab.env(), &mut Fake { drawn: 0 });
        assert!(rows.iter().any(|(step, outcome)| step == "plugin" && outcome.verb == "done" && outcome.detail.contains("kebbie-tasks")), "{rows:?}");
        assert_eq!(crate::agent::codex::installed(&lab.dir.join("codex"), "/opt/kebbie"), Installed::Current);
        let mut rerun = Guide::new(Facts::read(&lab.env()), Step::Agents);
        rerun.facts.codex = Some("0.163.0".to_string());
        rerun.go(Step::CodexPlugin);
        assert!(screen(&rerun).contains("✓ in place: kebbie's plugin at"), "{}", screen(&rerun));
    }

    #[test]
    fn a_machine_with_kebbies_codex_plugin_under_kebbie_at_kebbie_keeps_codex_on_and_apply_moves_it() {
        let lab = Lab::new("guide-old-id-codex");
        let plugins = lab.machine.claude_dir.join("plugins");
        std::fs::create_dir_all(&plugins).unwrap();
        std::fs::write(plugins.join("installed_plugins.json"), json!({ "plugins": { "kebbie@kebbie": [{ "version": "0.27.0" }] } }).to_string()).unwrap();
        let home = lab.dir.join("codex");
        crate::agent::codex::install(&home, "/opt/kebbie").unwrap();
        std::fs::create_dir_all(home.join("plugins/cache/kebbie/kebbie")).unwrap();
        std::fs::rename(home.join("plugins/cache/kebbie-tasks/kebbie/local"), home.join("plugins/cache/kebbie/kebbie/local")).unwrap();
        let mut guide = fresh(&lab);
        assert!(guide.answers.codex, "an opted-in machine is not shown Codex off");
        guide.go(Step::CodexPlugin);
        assert!(screen(&guide).contains("to update: kebbie's plugin in Codex is under kebbie@kebbie"), "{}", screen(&guide));
        let rows = apply(&guide.plan(), &lab.env(), &mut Fake { drawn: 0 });
        assert!(rows.iter().any(|(step, outcome)| step == "plugin" && outcome.verb == "done" && outcome.detail.contains("kebbie-tasks")), "{rows:?}");
        assert!(!home.join("plugins/cache/kebbie/kebbie/local").exists());
    }

    #[test]
    fn turning_codex_off_takes_kebbies_plugin_out_of_codex() {
        let lab = Lab::new("guide-codex-off");
        crate::agent::codex::install(&lab.dir.join("codex"), "/opt/kebbie").unwrap();
        let mut guide = fresh(&lab);
        press(&mut guide, &[KeyCode::Enter, KeyCode::Down, KeyCode::Char(' ')]);
        assert!(!guide.answers.codex && !guide.steps().contains(&Step::CodexPlugin));
        assert!(plan(&guide).contains(&"to do plugin kebbie's plugin out of Codex".to_string()), "{:?}", plan(&guide));
    }

    #[test]
    fn s_skips_a_step_and_back_keeps_what_was_chosen() {
        let lab = Lab::new("guide-skip");
        let mut guide = fresh(&lab);
        press(&mut guide, &[KeyCode::Enter, KeyCode::Char('s')]);
        assert_eq!(guide.step, Step::ClaudePlugin);
        press(&mut guide, &[KeyCode::Char('s')]);
        assert_eq!((guide.step, guide.answers.claude_plugin), (Step::ClaudeLogin, Way::Skip), "a skipped write is a skip, never kebbie's");
        guide.go(Step::Keys);
        press(&mut guide, &[KeyCode::Char('s'), KeyCode::Down, KeyCode::Char(' '), KeyCode::Left]);
        assert_eq!(guide.step, Step::Keys);
        press(&mut guide, &[KeyCode::Left]);
        assert_eq!(guide.step, Step::ClaudeUpdates, "← goes back through the agent's steps");
        guide.go(Step::Keys);
        press(&mut guide, &[KeyCode::Enter, KeyCode::Enter, KeyCode::Char('s')]);
        assert_eq!(guide.step, Step::Review);
        assert_eq!(guide.answers.chosen_sources(), [Source::Local, Source::Vault], "← and ⏎ keep the boxes ticked");
        guide.go(Step::Vault);
        press(&mut guide, &[KeyCode::Down, KeyCode::Char('s')]);
        assert_eq!(guide.answers.vault, 0, "a skip puts the step back to what the machine has");
        guide.go(Step::Review);
        let plan = plan(&guide);
        let row = |step: &str| plan.iter().find(|row| row.split_whitespace().nth(if row.starts_with("to do") { 2 } else { 1 }) == Some(step)).cloned().unwrap_or_default();
        assert!(row("keys").starts_with("skipped keys") && row("vault").starts_with("skipped vault") && row("signals") == "skipped signals stay on", "{plan:?}");
        assert!(row("plugin") == "skipped plugin kebbie's plugin stays out of Claude Code" && row("backlog").starts_with("to do backlog"), "{plan:?}");
        press(&mut guide, &[KeyCode::Left]);
        assert_eq!(guide.step, Step::Signals);
        let mut name = fresh(&lab);
        type_text(&mut name, "s");
        assert_eq!((name.step, name.answers.name.as_str()), (Step::Name, "s"), "s types in a text field");
        press(&mut name, &[KeyCode::Backspace, KeyCode::Enter]);
        assert!(screen(&name).contains("machine.name: must be a non-empty string"), "{}", screen(&name));
    }

    #[test]
    fn a_taken_herdr_key_is_named_with_a_free_one_offered() {
        let lab = Lab::new("guide-taken");
        std::fs::create_dir_all(lab.machine.herdr_config.parent().unwrap()).unwrap();
        std::fs::write(&lab.machine.herdr_config, "[keys]\nnew_tab = \"prefix+t\"\n").unwrap();
        let mut guide = fresh(&lab);
        guide.go(Step::Keys);
        let shown = screen(&guide);
        assert!(shown.contains("! prefix+t      open the board · taken by new_tab"), "{shown}");
        assert!(shown.contains("(•) Add them, leave prefix+t to new_tab  · bind the rest yourself later") && shown.contains("( ) Add them, the board on prefix+y  · free in your config") && shown.contains("( ) Skip  · add them by hand"), "{shown}");
        press(&mut guide, &[KeyCode::Down]);
        assert!(screen(&guide).contains("+ prefix+y      open the board · moved off prefix+t, which new_tab uses"));
        press(&mut guide, &[KeyCode::Enter, KeyCode::Char('s'), KeyCode::Char('s')]);
        assert_eq!(guide.step, Step::Review);
        let keys: Vec<Planned> = guide.plan().into_iter().filter(|row| row.step == "keys").collect();
        assert_eq!(format!("{} {}", keys[0].verb, keys[0].detail), "to do prefix+space, prefix+f, prefix+y in herdr");
        let rows = apply(&keys, &lab.env(), &mut Fake { drawn: 0 });
        assert_eq!(format!("{} {}", rows[0].1.verb, rows[0].1.detail), "done prefix+space, prefix+f, prefix+y added, restart herdr to use them");
        let herdr = lab.herdr();
        assert!(herdr.starts_with("[keys]\nnew_tab = \"prefix+t\"\n\n[[keys.command]]") && herdr.contains("key = \"prefix+y\"\ntype = \"plugin_action\"\ncommand = \"kebbie.board\""), "only the end of the file changes: {herdr}");
        let mut leave = fresh(&lab);
        std::fs::write(&lab.machine.herdr_config, "[keys]\nnew_tab = \"prefix+t\"\n").unwrap();
        leave.facts.keys = setup::key_plan(&lab.machine.herdr_config);
        leave.go(Step::Keys);
        press(&mut leave, &[KeyCode::Enter]);
        assert!(plan(&leave).contains(&"to do keys prefix+space, prefix+f in herdr, the board unbound".to_string()), "{:?}", plan(&leave));
    }

    #[test]
    fn an_s3_vault_masks_its_secret() {
        let lab = Lab::new("guide-s3");
        let mut guide = fresh(&lab);
        guide.go(Step::Vault);
        press(&mut guide, &[KeyCode::Up]);
        assert_eq!(guide.answers.vault, 2);
        for value in ["https://s3.example.com", "notes", "AKIA1", "hunter2"] {
            press(&mut guide, &[KeyCode::Down]);
            type_text(&mut guide, value);
        }
        let shown = screen(&guide);
        assert!(shown.contains("secret   •••••••") && !shown.contains("hunter2"), "{shown}");
        press(&mut guide, &[KeyCode::Enter]);
        assert_eq!(guide.step, Step::ClaudeUpdates);
        guide.go(Step::Review);
        let vault: Vec<Planned> = guide.plan().into_iter().filter(|row| row.step == "vault").collect();
        assert_eq!(format!("{} {}", vault[0].verb, vault[0].detail), "to do S3 bucket notes, through the vault plugin");
        let rows = apply(&vault[..1], &lab.env(), &mut Fake { drawn: 0 });
        assert_eq!(format!("{} {}", rows[0].1.verb, rows[0].1.detail), "done S3 bucket notes");
        assert!(!screen(&guide).contains("hunter2"), "the secret is never shown");
        assert!(std::fs::read_to_string(lab.dir.join("claude.stdin")).unwrap().contains("\"s3_secret_key\":\"hunter2\""));
    }

    // A machine with Codex, kebbie's plugin in it, the vault plugin on an S3 bucket and a config.toml of the user's own.
    fn codex_lab(name: &str) -> Lab {
        let lab = Lab::new(name);
        let codex = lab.dir.join("codex");
        crate::agent::codex::install(&codex, "/opt/kebbie").unwrap();
        std::fs::write(codex.join("config.toml"), "# mine\nmodel_reasoning_effort = \"high\"\n").unwrap();
        std::fs::write(codex.join("models_cache.json"), crate::agent::codex::CACHE_FIXTURE).unwrap();
        let root = lab.dir.join("vault-plugin");
        std::fs::create_dir_all(root.join("hooks")).unwrap();
        std::fs::write(root.join("hooks/hooks.json"), "uvx notes-vault-mcp==0.8.0 hook session-start").unwrap();
        std::fs::write(root.join(".mcp.json"), r#"{"mcpServers":{"vault":{"command":"uvx","args":["notes-vault-mcp==0.8.0"],"env":{"VAULT_PATH":"${user_config.vault_path}","S3_ENDPOINT":"${user_config.s3_endpoint}","S3_BUCKET":"${user_config.s3_bucket}","S3_ACCESS_KEY":"${user_config.s3_access_key}","S3_SECRET_KEY":"${user_config.s3_secret_key}"}}}}"#).unwrap();
        let plugins = lab.machine.claude_dir.join("plugins");
        std::fs::write(plugins.join("installed_plugins.json"), json!({ "plugins": { "kebbie@kebbie": [{ "version": "0.30.0" }], "vault@gronare": [{ "version": "0.2.19", "installPath": root }] } }).to_string()).unwrap();
        std::fs::write(lab.machine.claude_dir.join("settings.json"), json!({ "pluginConfigs": { "vault@gronare": { "options": { "s3_endpoint": "https://s3.example.com", "s3_bucket": "notes", "s3_access_key": "AKIA1", "s3_secret_key": "hunter2" } } } }).to_string()).unwrap();
        lab
    }

    fn wide(guide: &Guide) -> String {
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(130, 30)).unwrap();
        terminal.draw(|frame| guide.draw(frame)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        (0..buffer.area.height).map(|y| (0..buffer.area.width).map(|x| buffer[(x, y)].symbol().to_string()).collect::<String>().trim_end().to_string()).collect::<Vec<_>>().join("\n")
    }

    #[test]
    fn kebbie_setup_codex_walks_codexs_steps_and_apply_writes_only_what_kebbie_was_asked_to() {
        let lab = codex_lab("guide-codex");
        let mut guide = Guide::scoped(Facts::read(&lab.env()), Scope::Agent(Agent::Codex), Step::CodexPlugin);
        let shown = screen(&guide);
        assert!(shown.contains("kebbie setup  step 1 of 5") && shown.contains("kebbie setup codex · esc quits without writing") && shown.contains("✓ in place: kebbie's plugin at"), "{shown}");
        assert!(!shown.contains("Machine name") && !shown.contains("herdr keys") && !shown.contains("  Codex\n"), "only Codex's steps: {shown}");
        press(&mut guide, &[KeyCode::Enter]);
        println!("--- Codex approvals step ---\n{}", wide(&guide));
        let shown = screen(&guide);
        assert!(shown.contains("step 2 of 5") && shown.contains("Codex asks before each command that leaves its sandbox, kebbie's own included: report, tell, ask,"), "{shown}");
        assert!(shown.contains("▌ Add kebbie's rules file") && shown.contains("/codex/rules/kebbie.rules, 30 lines") && shown.contains("  I'll add it myself") && shown.contains("shows the file to copy") && shown.contains("  Skip") && shown.contains("Codex asks for each command"), "{shown}");
        assert!(shown.contains("↑ ↓ choose · ⏎ next · ← back · s skip · esc quit"), "{shown}");
        press(&mut guide, &[KeyCode::Down, KeyCode::Enter]);
        assert_eq!((guide.step, guide.answers.approvals), (Step::CodexVault, Way::Yourself));
        let shown = screen(&guide);
        assert!(shown.contains("step 3 of 5") && shown.contains("▌ Add the vault server") && shown.contains("[mcp_servers.vault] in") && shown.contains("The keys stay out of the file: Codex reads S3_ACCESS_KEY and S3_SECRET_KEY from the environment by name (env_vars)"), "{shown}");
        press(&mut guide, &[KeyCode::Enter]);
        let shown = screen(&guide);
        assert!(shown.contains("step 4 of 5") && shown.contains("▌ Ask me after U") && shown.contains("Update it with U") && shown.contains("Only tell me") && shown.contains("Codex profiles without a model follow Codex's own default, gpt-6.1-sol today."), "{shown}");
        press(&mut guide, &[KeyCode::Enter]);
        assert_eq!(guide.step, Step::Review);
        println!("--- review and apply ---\n{}", wide(&guide));
        let shown = screen(&guide);
        let config = format!("{}/codex/config.toml", lab.dir.display());
        assert_eq!(plan(&guide), [
            format!("already plugin kebbie's plugin at {}/codex/plugins/cache/kebbie-tasks/kebbie/local", lab.dir.display()),
            format!("yourself approvals {}/codex/rules/kebbie.rules", lab.dir.display()),
            format!("to do vault [mcp_servers.vault] in {config}, keys from the environment"),
            "to do updates ask me after U (update.agents = \"ask\" in kebbie's config.toml)".to_string(),
            "skipped model Codex profiles follow Codex's default".to_string(),
        ]);
        assert!(shown.contains(&format!("runs: adds 7 lines to the end of {config}; the rest of the file stays as it is")), "{shown}");
        assert_eq!(std::fs::read_to_string(&config).unwrap(), "# mine\nmodel_reasoning_effort = \"high\"\n", "nothing is written before ⏎");
        let planned = guide.plan();
        let rows = apply(&planned, &lab.env(), &mut Fake { drawn: 0 });
        guide.added = added(&lab.env(), &planned);
        (guide.applied, guide.done) = (Some(rows), true);
        let written = std::fs::read_to_string(&config).unwrap();
        assert!(written.starts_with("# mine\nmodel_reasoning_effort = \"high\"\n\n[mcp_servers.vault]\ncommand = \"uvx\"") && written.contains("env_vars = [\"S3_ACCESS_KEY\", \"S3_SECRET_KEY\"]"), "{written}");
        assert!(!written.contains("hunter2") && !written.contains("AKIA1"), "the keys never go in Codex's file");
        assert!(lab.config().contains("[update]\nagents = \"ask\"\n"), "{}", lab.config());
        assert!(!lab.dir.join("codex/rules").exists(), "the file the user adds is never written");
        println!("--- done, with what you add yourself ---\n{}", wide(&guide));
        let shown = screen(&guide);
        assert!(shown.contains("╭ Done ") && shown.contains("done     vault      added [mcp_servers.vault] to") && shown.contains("done     updates    update.agents = \"ask\""), "{shown}");
        assert!(!shown.contains("already  plugin"), "the done list names what changed: {shown}");
        assert!(shown.contains("You chose to add these yourself:") && shown.contains("yourself approvals  copy this file to") && shown.contains("prefix_rule(pattern=[\"/opt/kebbie\", \"peers\"], decision=\"allow\")") && shown.contains("… 29 more lines, also printed when you quit"), "{shown}");
        assert!(shown.contains("Until then Codex asks before each kebbie command, and kebbie doctor names the file as missing.") && shown.contains("Tasks keep running either way.") && shown.contains("any key closes"), "{shown}");
        let text = guide.text();
        assert_eq!(text.matches("prefix_rule(").count(), 30, "quitting prints the whole file: {text}");
        assert!(text.contains("yourself  approvals  copy this file to") && !text.contains("hunter2"), "{text}");
        assert!(matches!(guide.key(KeyEvent::from(KeyCode::Char('x'))), Next::Quit), "any key closes kebbie setup codex");
        let again = Guide::scoped(Facts::read(&lab.env()), Scope::Agent(Agent::Codex), Step::CodexVault);
        assert!(screen(&again).contains("✓ in place: [mcp_servers.vault] in"), "{}", screen(&again));
    }

    #[test]
    fn a_rules_file_in_place_reads_already_and_one_of_other_rules_reads_to_update() {
        let lab = codex_lab("guide-codex-rules");
        let mut guide = Guide::scoped(Facts::read(&lab.env()), Scope::Agent(Agent::Codex), Step::CodexApprovals);
        let rules = guide.facts.rules.clone();
        crate::agent::codex_setup::write_rules(&lab.dir.join("codex"), &rules).unwrap();
        guide = Guide::scoped(Facts::read(&lab.env()), Scope::Agent(Agent::Codex), Step::CodexApprovals);
        assert!(screen(&guide).contains("✓ in place:") && screen(&guide).contains("/codex/rules/kebbie.rules"), "{}", screen(&guide));
        assert!(plan(&guide).iter().any(|row| row.starts_with("already approvals")), "{:?}", plan(&guide));
        crate::agent::codex_setup::write_rules(&lab.dir.join("codex"), "prefix_rule(pattern=[\"kebbie\"], decision=\"allow\")\n").unwrap();
        guide = Guide::scoped(Facts::read(&lab.env()), Scope::Agent(Agent::Codex), Step::CodexApprovals);
        assert!(screen(&guide).contains("to update:") && screen(&guide).contains("is not the file this kebbie writes"), "{}", screen(&guide));
        assert!(plan(&guide).iter().any(|row| row.starts_with("to do approvals")));
        let rows = apply(&guide.plan(), &lab.env(), &mut Fake { drawn: 0 });
        assert!(rows.iter().any(|(step, outcome)| step == "approvals" && outcome.verb == "done"), "{rows:?}");
        assert_eq!(std::fs::read_to_string(lab.dir.join("codex/rules/kebbie.rules")).unwrap(), rules);
    }

    #[test]
    fn without_kebbies_vault_no_agent_has_a_vault_step() {
        let lab = Lab::new("guide-codex-no-vault");
        crate::agent::codex::install(&lab.dir.join("codex"), "/opt/kebbie").unwrap();
        let guide = Guide::scoped(Facts::read(&lab.env()), Scope::Agent(Agent::Codex), Step::CodexPlugin);
        assert_eq!(guide.steps(), [Step::CodexPlugin, Step::CodexApprovals, Step::CodexUpdates, Step::Review]);
        assert!(plan(&guide).iter().all(|row| !row.contains(" vault ")), "{:?}", plan(&guide));
        let claude = Guide::scoped(Facts::read(&lab.env()), Scope::Agent(Agent::Claude), Step::ClaudePlugin);
        assert_eq!(claude.steps(), [Step::ClaudePlugin, Step::ClaudeLogin, Step::ClaudeApprovals, Step::Vault, Step::ClaudeUpdates, Step::Review], "Claude's Vault step is where the vault is chosen");
        assert!(screen(&claude).contains("kebbie setup claude · esc quits without writing"));
    }

    #[test]
    fn kebbie_setup_codex_without_a_terminal_names_what_else_it_offers() {
        let lab = codex_lab("guide-codex-offers");
        let offers = offers(&lab.env(), Agent::Codex);
        assert_eq!(offers[0], "kebbie setup codex in a terminal also offers, each on your choice:");
        assert!(offers.iter().any(|line| line.contains("approvals") && line.contains("rules/kebbie.rules")) && offers.iter().any(|line| line.contains("vault") && line.contains("[mcp_servers.vault]")) && offers.iter().any(|line| line.contains("updates")), "{offers:?}");
        assert!(!lab.dir.join("codex/rules").exists() && !std::fs::read_to_string(lab.dir.join("codex/config.toml")).unwrap().contains("mcp_servers"), "naming them writes nothing");
    }

    #[test]
    fn a_doctor_warning_that_needs_the_users_choice_is_no_row() {
        let lab = Lab::new("guide-repairs");
        std::fs::create_dir_all(lab.dir.join("kebbie")).unwrap();
        std::fs::write(lab.dir.join("kebbie/config.toml"), "[task]\nsandbox = false\n").unwrap();
        std::fs::write(lab.machine.claude_dir.join("settings.json"), "{\"statusLine\": {\"type\": \"command\", \"command\": \"my-line\"}}").unwrap();
        let mut guide = fresh(&lab);
        assert_eq!(guide.facts.repairs, [Repair::KeeperStart], "a statusLine of the user's own and a task.sandbox = false they chose stay with kebbie doctor");
        guide.go(Step::Review);
        assert_eq!(plan(&guide).last().map(String::as_str), Some("to do keeper start it, so routines run without an open board"));
        assert!(plan(&guide).iter().all(|row| !row.contains("sandbox")), "{:?}", plan(&guide));
    }

    #[test]
    fn yes_takes_the_defaults_and_without_a_terminal_nothing_is_written_to_herdr() {
        let lab = Lab::new("guide-yes");
        let lines: Vec<String> = without_asking(&lab.env(), Asking::Yes).iter().map(|(step, outcome)| outcome.line(step)).collect();
        let verbs: Vec<&str> = lines.iter().map(|line| line.split_whitespace().next().unwrap_or_default()).collect();
        assert_eq!(verbs, ["done", "done", "done"], "{lines:?}");
        assert!(lines[2].contains("added prefix+space, prefix+f, prefix+t to") && lines[2].contains("restart herdr so it reads the new keys"), "{lines:?}");
        assert_eq!(lab.config(), "", "--yes leaves the optional steps as they are");
        assert!(!lab.machine.claude_dir.join("settings.json").exists(), "--yes edits no file of the user's that it did not show");
        let written = lab.herdr();
        let again: Vec<String> = without_asking(&lab.env(), Asking::Yes).iter().map(|(step, outcome)| outcome.line(step)).collect();
        assert!(again[0].starts_with("already") && again[1].starts_with("already") && again[2].starts_with("already"), "{again:?}");
        assert_eq!(lab.herdr(), written);
        let closed = Lab::new("guide-closed");
        let lines: Vec<String> = without_asking(&closed.env(), Asking::Closed).iter().map(|(step, outcome)| outcome.line(step)).collect();
        assert!(lines[2].starts_with("skipped") && lines[2].contains("no terminal to ask in") && lines[2].contains("kebbie setup --yes"), "{lines:?}");
        assert_eq!(closed.herdr(), "");
        assert!(!lab.dir.join("codex/plugins").exists(), "a machine without kebbie's Codex plugin gets none");
        crate::agent::codex::install(&lab.dir.join("codex"), "/old/kebbie").unwrap();
        let lines: Vec<String> = without_asking(&lab.env(), Asking::Yes).iter().map(|(step, outcome)| outcome.line(step)).collect();
        assert!(lines.iter().any(|line| line.starts_with("done") && line.contains("kebbie's Codex plugin updated: bin/kebbie-hook")), "an installed plugin follows kebbie: {lines:?}");
        assert_eq!(crate::agent::codex::installed(&lab.dir.join("codex"), "/opt/kebbie"), Installed::Current);
    }

    fn found(source: &str, command: &str) -> crate::vault_found::Found {
        let answer = crate::vault_cli::Probe { name: "mindin".to_string(), version: "0.14.0".to_string(), ..crate::vault_cli::Probe::default() };
        crate::vault_found::Found { source: source.to_string(), command: command.split(' ').map(str::to_string).collect(), answer: Some(answer) }
    }

    #[test]
    fn setup_shows_the_vault_commands_it_found_and_writes_the_one_picked_to_kebbies_own_config() {
        let lab = Lab::new("guide-vault-command");
        let mut facts = fresh(&lab).facts;
        assert!(!Guide::new(facts.clone(), Step::Name).steps().contains(&Step::VaultCommand), "nothing found, nothing asked");
        facts.vault_found = vec![found("the Claude Code vault plugin vault@gronare", "uvx mindin==0.14.0"), found("Codex's [mcp_servers.notes]", "uv run --project /src/mindin mindin")];
        let mut guide = Guide::new(facts.clone(), Step::VaultCommand);
        let shown = screen(&guide);
        assert!(shown.contains("Vault for kebbie") && shown.contains("the Claude Code vault plugin vault@gronare") && shown.contains("uvx mindin==0.14.0 · mindin 0.14.0") && shown.contains("Codex's [mcp_servers.notes]") && shown.contains("None"), "{shown}");
        assert_eq!(plan(&guide).iter().find(|row| row.contains(" vault cmd ")).map(String::as_str), Some("to do vault cmd vault.command = \"uvx mindin==0.14.0\" in kebbie's config.toml"), "the first one found is the default");
        press(&mut guide, &[KeyCode::Down, KeyCode::Enter]);
        let planned: Vec<Planned> = guide.plan().into_iter().filter(|row| row.step == "vault cmd").collect();
        apply(&planned, &lab.env(), &mut Fake { drawn: 0 });
        assert_eq!(lab.config(), "[vault]\ncommand = \"uv run --project /src/mindin mindin\"\n");
        assert!(!lab.dir.join("codex/config.toml").exists(), "no other tool's file is written");
        facts.vault_command = Some("uv run --project /src/mindin mindin".to_string());
        let again = Guide::new(facts.clone(), Step::VaultCommand);
        assert_eq!(plan(&again).iter().find(|row| row.contains(" vault cmd ")).map(String::as_str), Some("already vault cmd vault.command = \"uv run --project /src/mindin mindin\""));
        let mut none = Guide::new(facts.clone(), Step::VaultCommand);
        press(&mut none, &[KeyCode::Down, KeyCode::Enter]);
        let planned: Vec<Planned> = none.plan().into_iter().filter(|row| row.step == "vault cmd").collect();
        assert_eq!(planned[0].act, Act::VaultCommand(None));
        apply(&planned, &lab.env(), &mut Fake { drawn: 0 });
        assert!(!lab.config().contains("command"), "None takes kebbie's line out: {}", lab.config());
        facts.vault_found.clear();
        facts.vault_command = Some("uvx mindin".to_string());
        let kept = Guide::new(facts, Step::VaultCommand);
        assert!(screen(&kept).contains("Keep [vault] command") && kept.answers.vault_command == 0, "a line no tool shows is offered to keep");
    }

    #[test]
    fn a_run_that_asks_nothing_names_the_vault_command_it_found_and_writes_none() {
        let lab = Lab::new("guide-vault-unasked");
        let mut facts = fresh(&lab).facts;
        let config = lab.dir.join("kebbie/config.toml");
        assert_eq!(vault_command_unasked(&facts, &config), None);
        facts.vault_found = vec![found("mindin on PATH", "mindin"), found("Codex's [mcp_servers.notes]", "uvx mindin")];
        let said = vault_command_unasked(&facts, &config).unwrap();
        assert_eq!(said.verb, "yourself");
        assert!(said.detail.starts_with("mindin on PATH runs the vault: set vault.command = \"mindin\" under [vault] in ") && said.detail.ends_with("; kebbie setup in a terminal offers 1 more"), "{said:?}");
        facts.vault_command = Some("uvx mindin".to_string());
        assert_eq!(vault_command_unasked(&facts, &config).unwrap().verb, "already");
        assert_eq!(lab.config(), "");
    }

    fn check(level: crate::doctor::Level, name: &str, fact: &str, fix: &[&str]) -> crate::doctor::Check {
        crate::doctor::Check { level, name: name.to_string(), fact: fact.to_string(), fix: fix.iter().map(|fix| fix.to_string()).collect() }
    }

    #[test]
    fn the_end_names_the_path_line_and_the_file_for_the_users_shell_when_kebbie_is_not_on_path() {
        use crate::doctor::{Level, NOT_ON_PATH, ON_PATH};
        let missing = [check(Level::Warn, ON_PATH, NOT_ON_PATH, &[])];
        let step = |shell: &str| ending(None, &missing, shell)[4].clone();
        assert_eq!(step("/bin/zsh"), "  PATH                 add export PATH=\"$HOME/.local/bin:$PATH\" to ~/.zshrc, then open a new terminal");
        assert!(step("/usr/bin/fish").contains("fish_add_path ~/.local/bin to ~/.config/fish/config.fish"));
        assert!(step("").contains("to your shell's profile"));
        let found = [check(Level::Ok, ON_PATH, "~/.local/bin/kebbie → kebbie", &[])];
        assert!(!ending(None, &found, "/bin/zsh").iter().any(|line| line.contains("PATH")), "on PATH, no line");
    }

    #[test]
    fn the_end_counts_the_doctor_and_lists_only_what_needs_the_user_in_two_columns() {
        use crate::doctor::Level;
        let clean = ending(Some(Login::In), &[check(Level::Ok, "git", "/usr/bin/git", &[]), check(Level::Ok, "jq", "/usr/bin/jq", &[])], "/bin/zsh");
        assert_eq!(clean, ["", "Setup done · doctor: 2 ok", "", "Next steps", "  prefix+space         tell the lead what you want done"], "the lead is the one step left when nothing else is");
        let unchecked = ending(Some(Login::In), &[check(Level::Ok, "git", "/usr/bin/git", &[]), check(Level::Skip, "claude login", "login not checked", &[])], "/bin/zsh");
        assert_eq!(unchecked[1], "Setup done · doctor: 1 ok", "a check that got no answer is neither ok nor to look at");
        let checks = [check(Level::Ok, "git", "/usr/bin/git", &[]), check(Level::Fail, "jq", "jq is missing", &["macOS: brew install jq"]), check(Level::Warn, "status line", "statusLine runs \"my-line\"", &["have your statusLine command pipe its stdin to `kebbie statusline`"])];
        assert_eq!(ending(Some(Login::Out), &checks, "/bin/zsh"), [
            "",
            "Setup done · doctor: 1 ok, 2 to look at",
            "",
            "Next steps",
            "  codex login          Codex is not logged in, so codex tasks cannot start",
            "  kebbie doctor        says what is left and how to fix it",
            "  prefix+space         tell the lead what you want done",
        ], "a fix setup cannot make is one line to kebbie doctor, never its text");
        assert!(ending(Some(Login::Unknown), &[], "/bin/zsh").contains(&"  codex login          if you have not yet, so codex tasks can start".to_string()));
    }

    #[test]
    fn the_guide_opens_at_a_step_and_the_review_shows_each_line_as_it_is_applied() {
        let lab = Lab::new("guide-open");
        let mut guide = Guide::new(Facts::read(&lab.env()), Step::Vault);
        assert_eq!(guide.step, Step::Vault);
        assert!(screen(&guide).contains("▸ 6 Vault"), "{}", screen(&guide));
        guide.go(Step::Review);
        guide.applied = Some(vec![("link".to_string(), Outcome::new("done", "~/.local/bin/kebbie"))]);
        let shown = screen(&guide);
        assert!(shown.contains("done     link       ~/.local/bin/kebbie") && shown.contains("to do    plugin ") && shown.contains("applying…") && !shown.contains("Your board on your phone"), "{shown}");
        assert!(matches!(guide.key(KeyEvent::from(KeyCode::Char('h'))), Next::Stay), "keys wait while it applies");
    }

    fn finished(lab: &Lab) -> Guide {
        let mut guide = fresh(lab);
        guide.facts.repairs.clear();
        guide.visited = [true; Step::ALL.len()];
        guide.facts.name = "Hetzner devbox".to_string();
        guide.facts.name_set = true;
        guide.answers.name = guide.facts.name.clone();
        guide.go(Step::Review);
        guide.applied = Some(vec![
            ("link".to_string(), Outcome::new("done", "~/.local/bin/kebbie")),
            ("plugin".to_string(), Outcome::new("done", "kebbie in Claude Code and Codex")),
            ("machine".to_string(), Outcome::new("done", "\"Hetzner devbox\"")),
            ("keys".to_string(), Outcome::new("done", "prefix+f added, herdr reloaded")),
            ("backlog".to_string(), Outcome::new("already", "vault, linear")),
            ("vault".to_string(), Outcome::new("already", "S3 bucket obsidian-work")),
            ("signals".to_string(), Outcome::new("already", "on")),
        ]);
        let ok: Vec<crate::doctor::Check> = (0..36).map(|at| check(crate::doctor::Level::Ok, &format!("check {at}"), "fine", &[])).collect();
        guide.ending = ending(Some(Login::Out), &ok, "/bin/zsh");
        guide.done = true;
        guide
    }

    #[test]
    fn the_finished_guide_offers_the_hub_in_a_box_and_h_connects_while_any_other_key_opens_the_board() {
        let lab = Lab::new("guide-finished");
        let mut guide = finished(&lab);
        let shown = screen(&guide);
        for line in [
            "Setup done · doctor: 36 ok",
            "codex login          Codex is not logged in, so codex tasks cannot start",
            "prefix+space         tell the lead what you want done",
            "╭ Your board on your phone ",
            "│ h  connect now     later: kebbie hub connect, or H on the board",
            "│ kebbie.dev/hub",
            "any key opens your board",
        ] {
            assert!(shown.contains(line), "{line:?} missing from:\n{shown}");
        }
        assert!(shown.contains("Follow tasks and answer questions from anywhere, end-to-end") && shown.contains("Free on hub.kebbie.dev, or run your own."), "{shown}");
        assert!(guide.text().ends_with("  prefix+space         tell the lead what you want done\n\nYour board on your phone: kebbie hub connect, or H on the board\nkebbie.dev/hub"), "{}", guide.text());
        assert!(!guide.said(false).contains("kebbie.dev/hub"), "the connect that h runs follows the summary instead");
        assert!(matches!(guide.key(KeyEvent::from(KeyCode::Char('h'))), Next::Connect));
        assert!(matches!(guide.key(KeyEvent::from(KeyCode::Char('x'))), Next::Board), "any other key opens the board");
        guide.facts.hub = Some("hub.kebbie.dev".to_string());
        let shown = screen(&guide);
        assert!(!shown.contains("Your board on your phone") && shown.contains("any key opens your board"), "a connected machine is offered nothing: {shown}");
        assert!(!guide.text().contains("kebbie.dev/hub"));
        assert!(matches!(guide.key(KeyEvent::from(KeyCode::Char('h'))), Next::Board), "h is any other key once connected");
    }

    #[test]
    fn a_board_opened_before_setup_starts_in_the_guide() {
        let dir = crate::sh::test_dir("first-run");
        assert!(first_run(&dir, true), "no installed_plugins.json, as after herdr plugin install alone");
        std::fs::create_dir_all(dir.join("plugins")).unwrap();
        std::fs::write(dir.join("plugins/installed_plugins.json"), json!({ "plugins": { "vault@gronare": [{ "version": "0.3.0" }] } }).to_string()).unwrap();
        assert!(first_run(&dir, true), "another plugin is not kebbie's");
        assert!(!first_run(&dir.join("missing"), false), "without a terminal the board never asks");
        std::fs::write(dir.join("plugins/installed_plugins.json"), json!({ "plugins": { "kebbie@kebbie": [{ "version": "0.29.1" }] } }).to_string()).unwrap();
        assert!(!first_run(&dir, true), "set up: the board opens straight away");
        let main = include_str!("main.rs");
        assert!(main.contains("Some(\"board\") => setup_guide::board("), "kebbie board, which herdr's startup, the lead Space and the board popup run, goes through the first-run check");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
