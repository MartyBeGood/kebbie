use std::path::{Path, PathBuf};

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Margin, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::agent::Agent;
use crate::agent::codex::CachedModel;
use crate::board::widgets::{BLUE, FAINT, GREEN, MANTLE, MAUVE, MUTED, PEACH, RED, SUBTEXT, SURFACE, TEAL, TEXT, YELLOW, bold, cut, modal, style, text_width, wrap};
use crate::config::{self, Entry, Kind, Layer, Scope};
use crate::config_edit::{self, Set};
use crate::profiles::{self, Edited, Profile};
use crate::sh::Result;

pub enum Effect {
    Handled,
    Left,
    Open(PathBuf),
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Tab {
    Machine,
    Repo,
}

// The page's sections, by the table a key sits in; a key's own line comes from kebbie config's entries.
const SECTIONS: [(&str, &[&str]); 8] = [
    ("Tasks", &["task"]),
    ("Board", &["board"]),
    ("Worktrees", &["worktree"]),
    ("Review & docs", &["review", "ci", "comments", "docs"]),
    (PROFILES, &[]),
    ("Updates", &["update", "restart", "keeper", "recover"]),
    ("This machine", &["machine", "hub", "notify", "vault"]),
    ("Reports", &["report", "issues"]),
];

const PROFILES: &str = "Profiles";
const CLAUDE_MODELS: [(&str, &str); 4] = [("opus", "the newest Opus"), ("sonnet", "the newest Sonnet"), ("haiku", "the newest Haiku"), ("fable", "the newest Fable")];

fn section_of(key: &str) -> usize {
    let table = key.split('.').next().unwrap_or_default();
    SECTIONS.iter().position(|(_, tables)| tables.contains(&table)).unwrap_or(SECTIONS.len() - 1)
}

#[derive(Clone, Debug, PartialEq)]
enum Change {
    Set(Set),
    Remove,
    Copy(&'static str),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Typed {
    Value,
    SkillName,
    SkillFile,
    Model,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Field {
    Model,
    Effort,
}

impl Field {
    fn key(self) -> &'static str {
        match self {
            Field::Model => "model",
            Field::Effort => "effort",
        }
    }
}

#[derive(Clone, Debug)]
struct Choice {
    label: String,
    about: String,
    set: Option<Option<String>>,
    now: bool,
    dim: bool,
    gap: bool,
}

impl Choice {
    fn value(label: &str, about: &str, now: bool) -> Self {
        Choice { label: label.to_string(), about: about.to_string(), set: Some(Some(label.to_string())), now, dim: false, gap: false }
    }
}

struct Chooser {
    field: Field,
    title: String,
    head: String,
    choices: Vec<Choice>,
    index: usize,
    foot: Vec<String>,
}

#[derive(Clone, Debug)]
struct Pick {
    label: String,
    about: String,
    then: Then,
    now: bool,
}

#[derive(Clone, Debug)]
enum Then {
    Write(Change),
    Type(Typed),
}

enum Mode {
    Normal,
    Pick { picks: Vec<Pick>, index: usize },
    Edit { typed: Typed, buffer: String },
    Filter { buffer: String },
    Team { key: &'static str, change: Change },
    Choose(Box<Chooser>),
}

pub struct Paths {
    pub machine: PathBuf,
    pub repos_dir: PathBuf,
    pub claude_home: PathBuf,
    pub kebbie: PathBuf,
    pub profiles: PathBuf,
    pub codex_home: PathBuf,
}

impl Paths {
    pub fn current() -> Self {
        let home = PathBuf::from(std::env::var("HOME").unwrap_or_default());
        Paths {
            machine: config::path(),
            repos_dir: config::repos_dir(),
            claude_home: crate::agent::claude::home().unwrap_or_else(|| home.join(".claude")),
            kebbie: crate::lead::install_root().unwrap_or_else(|_| home.join(".local/share/kebbie/current")),
            profiles: profiles::dir(),
            codex_home: crate::agent::codex::home().unwrap_or_else(|| home.join(".codex")),
        }
    }
}

pub struct Page {
    paths: Paths,
    repo: Option<PathBuf>,
    tab: Tab,
    section: usize,
    row: usize,
    filter: String,
    mode: Mode,
    said: Option<(String, Color)>,
    entries: Vec<Entry>,
    error: Option<String>,
    agents: Agents,
}

#[derive(Default)]
struct Agents {
    profiles: Vec<Profile>,
    ignored: Vec<String>,
    codex_models: Option<Vec<CachedModel>>,
    codex_default: Option<String>,
    claude_default: Option<String>,
    review_model: Option<(String, Layer)>,
}

impl Agents {
    fn load(paths: &Paths, entries: &[Entry]) -> Self {
        let (profiles, ignored) = profiles::list_in(&paths.profiles);
        Agents {
            profiles, ignored,
            review_model: entries.iter().find(|entry| entry.key == "review.model" && entry.layer != Layer::Default).map(|entry| (edit_text(&entry.value), entry.layer)),
            codex_models: crate::agent::codex::cached_models(&paths.codex_home),
            codex_default: crate::agent::codex::configured_model(&paths.codex_home),
            claude_default: crate::agent::claude::configured_model(&paths.claude_home),
        }
    }

    fn default_model(&self, agent: Agent) -> Option<&str> {
        match agent {
            Agent::Codex => self.codex_default.as_deref(),
            Agent::Claude => self.claude_default.as_deref(),
        }
    }

    fn review_model(&self, profile: &Profile) -> Option<&str> {
        self.review_model.as_ref().filter(|_| profile.name == profiles::REVIEWER).map(|(model, _)| model.as_str())
    }

    fn own_model(&self, profile: &Profile) -> Option<String> {
        self.review_model(profile).map(str::to_string).or_else(|| profile.model.clone())
    }

    fn model(&self, profile: &Profile) -> Option<String> {
        self.own_model(profile).or_else(|| self.default_model(profile.agent).map(str::to_string))
    }

    fn offered(&self, model: &str) -> Option<bool> {
        self.codex_models.as_ref().map(|models| models.iter().any(|cached| cached.listed && cached.slug == model))
    }

    fn cached(&self, model: &str) -> Option<&CachedModel> {
        self.codex_models.iter().flatten().find(|cached| cached.slug == model)
    }

    fn warning(&self, profile: &Profile) -> Option<String> {
        let model = self.model(profile).filter(|_| profile.agent == Agent::Codex)?;
        if self.offered(&model) != Some(false) {
            return None;
        }
        let from = if self.own_model(profile).is_none() { " from Codex's config.toml" } else { "" };
        Some(format!("⚠ {} runs {model}{from}, which Codex does not offer to your account; pick a model for it with m.", profile.name))
    }
}

fn edit_text(value: &str) -> String {
    match format!("v = {value}").parse::<toml::Table>().ok().and_then(|mut table| table.remove("v")) {
        Some(toml::Value::String(text)) => text,
        Some(toml::Value::Array(items)) => items.iter().filter_map(toml::Value::as_str).collect::<Vec<_>>().join(", "),
        Some(toml::Value::Integer(number)) => number.to_string(),
        Some(toml::Value::Boolean(on)) => on.to_string(),
        _ => String::new(),
    }
}

fn typed_list(text: &str) -> Result<Vec<String>> {
    let text = text.trim();
    if text.starts_with('[') {
        let parsed: toml::Table = format!("v = {text}").parse().map_err(|_| "a list is \"a\", \"b\" or [\"a\", \"b\"]".to_string())?;
        return parsed["v"].as_array().map(|items| items.iter().filter_map(toml::Value::as_str).map(str::to_string).collect()).ok_or_else(|| "a list of strings".to_string());
    }
    Ok(text.split(',').map(str::trim).filter(|item| !item.is_empty()).map(str::to_string).collect())
}

fn default_skill(key: &str) -> &'static str {
    if key == "task.rules_skill" { config::DEFAULT_RULES_SKILL } else { config::DEFAULT_SETUP_SKILL }
}

fn home_relative(path: &Path) -> String {
    crate::area_page::home_relative(path)
}

fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to).map_err(|error| format!("{}: {error}", to.display()))?;
    for entry in std::fs::read_dir(from).map_err(|error| format!("{}: {error}", from.display()))?.flatten() {
        let target = to.join(entry.file_name());
        if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target).map_err(|error| format!("{}: {error}", target.display()))?;
        }
    }
    Ok(())
}

// kebbie's skill as a starting point of the user's own; one already there is kept, never overwritten.
pub fn copy_skill(kebbie: &Path, claude_home: &Path, skill: &str) -> Result<(PathBuf, bool)> {
    let to = claude_home.join("skills").join(skill);
    if to.exists() {
        return Ok((to, false));
    }
    copy_dir(&kebbie.join("skills").join(skill), &to)?;
    Ok((to, true))
}

impl Page {
    pub fn new(paths: Paths, repo: Option<PathBuf>) -> Self {
        Page { paths, repo, tab: Tab::Machine, section: 0, row: 0, filter: String::new(), mode: Mode::Normal, said: None, entries: Vec::new(), error: None, agents: Agents::default() }
    }

    pub fn is_normal(&self) -> bool {
        matches!(self.mode, Mode::Normal)
    }

    pub fn open(&mut self, repo: Option<PathBuf>) {
        if repo.is_some() && repo != self.repo {
            self.repo = repo;
            self.tab = Tab::Machine;
        }
        self.load();
    }

    fn repo_file(&self) -> Option<PathBuf> {
        self.repo.as_deref().map(|repo| config::repo_file_in(&self.paths.repos_dir, repo))
    }

    fn file(&self) -> PathBuf {
        match (self.tab, self.repo_file()) {
            (Tab::Repo, Some(file)) => file,
            _ => self.paths.machine.clone(),
        }
    }

    fn linked(&self) -> bool {
        self.tab == Tab::Repo && std::fs::symlink_metadata(self.file()).is_ok_and(|meta| meta.file_type().is_symlink())
    }

    fn repo_name(&self) -> String {
        self.repo_file().and_then(|file| file.file_stem().map(|stem| stem.to_string_lossy().to_string())).unwrap_or_default()
    }

    pub fn load(&mut self) {
        let repo = if self.tab == Tab::Repo { self.repo_file() } else { None };
        match config::resolve_at(self.paths.machine.clone(), repo) {
            Ok(loaded) => {
                let tab = self.tab;
                self.entries = config::entries(&loaded).into_iter().filter(|entry| match tab { Tab::Machine => entry.scope != Scope::Repo, Tab::Repo => entry.scope != Scope::Machine }).collect();
                self.error = None;
            }
            Err(error) => {
                self.entries = Vec::new();
                self.error = Some(error);
            }
        }
        self.agents = Agents::load(&self.paths, &self.entries);
        let sections = self.sections();
        self.section = self.section.min(sections.len().saturating_sub(1));
        let rows = sections.get(self.section).map_or(0, |(_, rows)| rows.len());
        self.row = self.row.min(rows.saturating_sub(1));
    }

    fn filter_text(&self) -> &str {
        match &self.mode {
            Mode::Filter { buffer } => buffer,
            _ => &self.filter,
        }
    }

    fn sections(&self) -> Vec<(String, Vec<usize>)> {
        let filter = self.filter_text().to_lowercase();
        if !filter.is_empty() {
            let found = (0..self.entries.len()).filter(|index| self.entries[*index].key.contains(&filter) || self.entries[*index].about.to_lowercase().contains(&filter)).collect();
            return vec![(format!("Search: {filter}"), found)];
        }
        SECTIONS.iter().enumerate().map(|(at, (name, _))| {
            let rows = match (*name, self.tab) {
                (PROFILES, Tab::Machine) if self.error.is_none() => (0..self.agents.profiles.len()).collect(),
                (PROFILES, _) => Vec::new(),
                _ => (0..self.entries.len()).filter(|index| section_of(self.entries[*index].key) == at).collect::<Vec<_>>(),
            };
            (name.to_string(), rows)
        }).filter(|(_, rows)| !rows.is_empty()).collect()
    }

    fn on_profiles(&self) -> bool {
        self.filter_text().is_empty() && self.tab == Tab::Machine && self.sections().get(self.section).is_some_and(|(name, _)| name == PROFILES)
    }

    fn profile(&self) -> Option<Profile> {
        if !self.on_profiles() {
            return None;
        }
        self.agents.profiles.get(self.row).cloned()
    }

    fn current(&self) -> Option<Entry> {
        if self.on_profiles() {
            return None;
        }
        let sections = self.sections();
        sections.get(self.section).and_then(|(_, rows)| rows.get(self.row)).map(|index| self.entries[*index].clone())
    }

    fn say(&mut self, text: String, color: Color) {
        self.said = Some((text, color));
    }

    fn picks(&self, entry: &Entry) -> Vec<Pick> {
        let now = edit_text(&entry.value);
        match &entry.kind {
            Kind::Choice(options) => options.iter().map(|option| {
                let now = *option == now || (*option == "none" && entry.value == "none");
                Pick {
                    label: if *option == "none" { "none".to_string() } else { format!("{option:?}") },
                    about: if now { "now".to_string() } else { String::new() },
                    then: Then::Write(if *option == "none" { Change::Remove } else { Change::Set(Set::Text(option.to_string())) }),
                    now,
                }
            }).collect(),
            Kind::Skill => {
                let default = default_skill(entry.key);
                let skill = default.split_once(':').map_or(default, |(_, skill)| skill);
                vec![
                    Pick { label: format!("{default:?}"), about: "built in".to_string(), then: Then::Write(Change::Set(Set::Text(default.to_string()))), now: false },
                    Pick { label: "Your skill, by name".to_string(), about: "<skill> or <plugin>:<skill>".to_string(), then: Then::Type(Typed::SkillName), now: false },
                    Pick { label: "A Markdown file, by path".to_string(), about: "an absolute or ~ path to a .md file".to_string(), then: Then::Type(Typed::SkillFile), now: false },
                    Pick { label: format!("Copy kebbie's to {}/{skill} and edit it", home_relative(&self.paths.claude_home.join("skills"))), about: format!("then the key names \"{skill}\""), then: Then::Write(Change::Copy(skill)), now: false },
                ]
            }
            _ => Vec::new(),
        }
    }

    pub fn paste(&mut self, text: &str) -> bool {
        match &mut self.mode {
            Mode::Edit { buffer, .. } | Mode::Filter { buffer } => {
                buffer.push_str(text.trim_end_matches(['\n', '\r']));
                true
            }
            _ => false,
        }
    }

    pub fn handle(&mut self, key: KeyEvent) -> Effect {
        match std::mem::replace(&mut self.mode, Mode::Normal) {
            Mode::Normal => return self.handle_normal(key),
            Mode::Filter { mut buffer } => match key.code {
                KeyCode::Enter => {
                    self.filter = buffer;
                    (self.section, self.row) = (0, 0);
                }
                KeyCode::Esc => self.filter.clear(),
                KeyCode::Backspace => {
                    buffer.pop();
                    self.mode = Mode::Filter { buffer };
                    (self.section, self.row) = (0, 0);
                }
                KeyCode::Char(character) => {
                    buffer.push(character);
                    self.mode = Mode::Filter { buffer };
                    (self.section, self.row) = (0, 0);
                }
                _ => self.mode = Mode::Filter { buffer },
            },
            Mode::Pick { picks, index } => match key.code {
                KeyCode::Up | KeyCode::Char('k') => self.mode = Mode::Pick { index: index.saturating_sub(1), picks },
                KeyCode::Down | KeyCode::Char('j') => self.mode = Mode::Pick { index: (index + 1).min(picks.len().saturating_sub(1)), picks },
                KeyCode::Enter => match (picks.get(index).map(|pick| pick.then.clone()), self.current()) {
                    (Some(Then::Write(change)), Some(entry)) => self.request(entry.key, change),
                    (Some(Then::Type(typed)), Some(entry)) => {
                        let now = edit_text(&entry.value);
                        let keep = match typed { Typed::SkillFile => config::skill_path(&now).is_some(), _ => now != default_skill(entry.key) && config::skill_path(&now).is_none() };
                        self.mode = Mode::Edit { typed, buffer: if keep { now } else { String::new() } };
                    }
                    _ => {}
                },
                KeyCode::Esc | KeyCode::Char('q') => {}
                _ => self.mode = Mode::Pick { picks, index },
            },
            Mode::Edit { typed, mut buffer } => match key.code {
                KeyCode::Enter => self.save_typed(typed, &buffer),
                KeyCode::Esc => {}
                KeyCode::Backspace => {
                    buffer.pop();
                    self.mode = Mode::Edit { typed, buffer };
                }
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => self.mode = Mode::Edit { typed, buffer: String::new() },
                KeyCode::Char(character) => {
                    buffer.push(character);
                    self.mode = Mode::Edit { typed, buffer };
                }
                _ => self.mode = Mode::Edit { typed, buffer },
            },
            Mode::Choose(mut chooser) => match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    chooser.index = chooser.index.saturating_sub(1);
                    self.mode = Mode::Choose(chooser);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    chooser.index = (chooser.index + 1).min(chooser.choices.len().saturating_sub(1));
                    self.mode = Mode::Choose(chooser);
                }
                KeyCode::Enter => match chooser.choices.get(chooser.index).map(|choice| choice.set.clone()) {
                    Some(Some(value)) => self.set_field(chooser.field, value.as_deref()),
                    Some(None) => self.mode = Mode::Edit { typed: Typed::Model, buffer: self.profile().and_then(|profile| self.agents.own_model(&profile)).unwrap_or_default() },
                    None => {}
                },
                KeyCode::Esc | KeyCode::Char('q') => {}
                _ => self.mode = Mode::Choose(chooser),
            },
            Mode::Team { key: name, change } => match key.code {
                KeyCode::Char('t') => self.apply(name, change, false),
                KeyCode::Char('o') => self.apply(name, change, true),
                KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('q') => {}
                _ => self.mode = Mode::Team { key: name, change },
            },
        }
        Effect::Handled
    }

    fn handle_normal(&mut self, key: KeyEvent) -> Effect {
        if self.on_profiles() {
            match key.code {
                KeyCode::Char('m') => return self.choose(Field::Model),
                KeyCode::Char('e') => return self.choose(Field::Effort),
                KeyCode::Char('d') => {
                    let reviewed = self.profile().and_then(|profile| self.agents.review_model(&profile).map(str::to_string)).is_some();
                    let removed = reviewed.then(|| {
                        self.request("review.model", Change::Remove);
                        self.said.take().map(|(text, _)| text).unwrap_or_default()
                    });
                    self.write_profile(&[("model", None), ("effort", None)]);
                    if let Some(removed) = removed && let Some((text, color)) = self.said.take() {
                        self.say(format!("{removed} · {text}"), if removed.starts_with('✗') { RED } else { color });
                    }
                    return Effect::Handled;
                }
                KeyCode::Enter => return self.open_profile(),
                KeyCode::Char(' ') => return Effect::Handled,
                _ => {}
            }
        }
        let sections = self.sections();
        let rows = sections.get(self.section).map_or(0, |(_, rows)| rows.len());
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.row = if self.row == 0 { rows.saturating_sub(1) } else { self.row - 1 };
                self.said = None;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.row = if self.row + 1 >= rows { 0 } else { self.row + 1 };
                self.said = None;
            }
            KeyCode::Left | KeyCode::Right if !sections.is_empty() => {
                let count = sections.len();
                self.section = if key.code == KeyCode::Left { (self.section + count - 1) % count } else { (self.section + 1) % count };
                self.row = 0;
                self.said = None;
            }
            KeyCode::Tab | KeyCode::BackTab => self.switch_tab(),
            KeyCode::Char('/') => self.mode = Mode::Filter { buffer: self.filter.clone() },
            KeyCode::Char(' ') => match self.current() {
                Some(entry) if entry.kind == Kind::Bool => self.request(entry.key, Change::Set(Set::Bool(entry.value != "true"))),
                Some(entry) => self.say(format!("space toggles a true or false key; ⏎ changes {}", entry.key), MUTED),
                None => {}
            },
            KeyCode::Enter => self.enter(),
            KeyCode::Char('d') => {
                if let Some(entry) = self.current() {
                    self.request(entry.key, Change::Remove);
                }
            }
            KeyCode::Esc if !self.filter.is_empty() => {
                self.filter.clear();
                (self.section, self.row) = (0, 0);
            }
            KeyCode::Esc | KeyCode::Char('q') => return Effect::Left,
            _ => {}
        }
        Effect::Handled
    }

    fn switch_tab(&mut self) {
        if self.repo.is_none() {
            return self.say("No repo here: select a task on the board, or run kebbie config edit --repo <path>".to_string(), PEACH);
        }
        self.tab = if self.tab == Tab::Machine { Tab::Repo } else { Tab::Machine };
        (self.section, self.row, self.said) = (0, 0, None);
        self.load();
    }

    fn enter(&mut self) {
        let Some(entry) = self.current() else { return };
        match entry.kind {
            Kind::Bool => self.request(entry.key, Change::Set(Set::Bool(entry.value != "true"))),
            Kind::Choice(_) | Kind::Skill => {
                let picks = self.picks(&entry);
                let index = picks.iter().position(|pick| pick.now).unwrap_or(0);
                self.mode = Mode::Pick { picks, index };
            }
            Kind::Secret => self.say(format!("{} is never shown here; set it in {} by hand", entry.key, home_relative(&self.paths.machine)), MUTED),
            Kind::Text | Kind::Number | Kind::List => self.mode = Mode::Edit { typed: Typed::Value, buffer: edit_text(&entry.value) },
        }
    }

    fn save_typed(&mut self, typed: Typed, buffer: &str) {
        let text = buffer.trim().to_string();
        if typed == Typed::Model {
            return self.set_field(Field::Model, Some(&text));
        }
        let Some(entry) = self.current() else { return };
        let set = match (typed, &entry.kind) {
            (Typed::Value, Kind::Number) => match text.parse::<i64>() {
                Ok(number) => Set::Number(number),
                Err(_) => return self.say(format!("✗ {}: \"{text}\" is not a whole number", entry.key), RED),
            },
            (Typed::Value, Kind::List) => match typed_list(&text) {
                Ok(items) => Set::List(items),
                Err(error) => return self.say(format!("✗ {}: {error}", entry.key), RED),
            },
            (Typed::SkillFile, _) if config::skill_path(&text).is_none() => return self.say(format!("✗ {}: a file goes by its absolute or ~ path, such as ~/code/team/RULES.md", entry.key), RED),
            _ => Set::Text(text),
        };
        self.request(entry.key, Change::Set(set));
    }

    fn request(&mut self, key: &'static str, change: Change) {
        if self.linked() {
            self.mode = Mode::Team { key, change };
            return;
        }
        self.apply(key, change, false);
    }

    fn apply(&mut self, key: &'static str, change: Change, replace_link: bool) {
        let file = self.file();
        let machine = self.tab == Tab::Machine || self.repo.is_none();
        let name = file.file_name().map(|name| name.to_string_lossy().to_string()).unwrap_or_default();
        let shown = if machine { name.clone() } else { format!("repos/{name}") };
        let (change, copied) = match change {
            Change::Copy(skill) => match copy_skill(&self.paths.kebbie, &self.paths.claude_home, skill) {
                Ok((to, true)) => (Change::Set(Set::Text(skill.to_string())), format!(" · copied kebbie's skill to {}: edit {}", home_relative(&to), home_relative(&to.join("SKILL.md")))),
                Ok((to, false)) => (Change::Set(Set::Text(skill.to_string())), format!(" · {} was already there, so nothing was copied: edit its SKILL.md", home_relative(&to))),
                Err(error) => return self.say(format!("✗ copying kebbie's {skill}: {error}"), RED),
            },
            other => (other, String::new()),
        };
        let written = match &change {
            Change::Set(set) => config_edit::write(&file, machine, replace_link, |text| config_edit::set(text, key, set)).map(|()| format!("✓ wrote {key} = {} to {shown}", set.shown())),
            _ => config_edit::write(&file, machine, replace_link, |text| config_edit::remove(text, key)).map(|()| format!("✓ removed {key} from {shown}; the layer below applies")),
        };
        match written {
            Ok(said) => {
                let check = if machine { crate::doctor::config_file(&file).0 } else { crate::doctor::repo_file(&file).0 };
                let level = match check.level { crate::doctor::Level::Ok => "ok", crate::doctor::Level::Warn => "warn", crate::doctor::Level::Fail => "fail" };
                let fact = check.fact.replace(&file.display().to_string(), &shown);
                let color = if check.level == crate::doctor::Level::Ok { GREEN } else { PEACH };
                let linked = if replace_link { " (now a file of this machine's own)" } else { "" };
                self.say(format!("{said}{linked}{copied} · doctor: {level}, {fact}"), color);
            }
            Err(error) => self.say(format!("✗ {}", error.lines().next().unwrap_or_default()), RED),
        }
        self.load();
    }

    fn shown_file(&self, file: &Path) -> String {
        let name = file.file_name().map(|name| name.to_string_lossy().to_string()).unwrap_or_default();
        format!("profiles/{name}")
    }

    fn built_in_without_file(&self, profile: &Profile) -> bool {
        profiles::built_in(&profile.name).is_some() && profile.file.is_none()
    }

    fn set_field(&mut self, field: Field, value: Option<&str>) {
        if field == Field::Model && self.profile().is_some_and(|profile| profile.name == profiles::REVIEWER) {
            return self.request("review.model", value.map_or(Change::Remove, |model| Change::Set(Set::Text(model.to_string()))));
        }
        self.write_profile(&[(field.key(), value)]);
    }

    fn foot(&self, profile: &Profile, field: Field) -> Vec<String> {
        let key = field.key();
        if field == Field::Model && profile.name == profiles::REVIEWER {
            let file = home_relative(&self.file());
            return vec![format!("writes review.model = \"…\" in {file}, ahead of the reviewer profile's model"), "a review already running keeps its model; the next review takes it".to_string()];
        }
        if self.built_in_without_file(profile) {
            return vec![format!("{name} is built in: setting a {key} writes profiles/{name}.toml with {key} alone, and the built-in's role and rules stay as they are.", name = profile.name), "d on the row removes that file again.".to_string(), format!("a task already running keeps its {key}; the next start takes it")];
        }
        let file = profile.file.as_deref().map(|file| self.shown_file(file)).unwrap_or_default();
        let file = file.trim_start_matches("profiles/");
        vec![format!("writes {key} = \"…\" in {file}, the rest stays as it is"), format!("a task already running keeps its {key}; the next start takes it")]
    }

    fn choose(&mut self, field: Field) -> Effect {
        let Some(profile) = self.profile() else { return Effect::Handled };
        let agents = &self.agents;
        let built_in = if self.built_in_without_file(&profile) { format!("; {} is built in", profile.name) } else { String::new() };
        let (title, head, choices) = match (field, profile.agent) {
            (Field::Model, Agent::Codex) => {
                let cache = home_relative(&self.paths.codex_home.join("models_cache.json"));
                let listed: Vec<&CachedModel> = agents.codex_models.iter().flatten().filter(|model| model.listed).collect();
                let head = if agents.codex_models.is_some() { format!("Codex's own list for your account ({cache})") } else { format!("no list from Codex at {cache}: Codex writes it when it starts") };
                let mut choices: Vec<Choice> = listed.iter().map(|model| Choice { label: model.name.clone(), about: model.slug.clone(), set: Some(Some(model.slug.clone())), now: profile.model.as_deref() == Some(model.slug.as_str()), dim: false, gap: false }).collect();
                let default = match agents.codex_default.as_deref() {
                    Some(model) if agents.offered(model) == Some(false) => format!("{model}, not offered to you ⚠"),
                    Some(model) => model.to_string(),
                    None => "none in Codex's config.toml: Codex picks".to_string(),
                };
                choices.push(Choice { label: "Codex's default".to_string(), about: default, set: Some(None), now: profile.model.is_none(), dim: true, gap: !choices.is_empty() });
                choices.push(Choice { label: "Type a model…".to_string(), about: "any id Codex accepts".to_string(), set: None, now: false, dim: true, gap: false });
                (format!("{} · model", profile.name), head, choices)
            }
            (Field::Model, Agent::Claude) => {
                let settings = home_relative(&self.paths.claude_home.join("settings.json"));
                let default = match agents.claude_default.as_deref() {
                    Some(model) => format!("{model}, from {settings}"),
                    None => format!("none in {settings}: Claude Code picks"),
                };
                let reviewer = profile.name == profiles::REVIEWER;
                let current = if reviewer { agents.review_model(&profile).map(str::to_string) } else { profile.model.clone() };
                let (label, default) = match (reviewer, &profile.model) {
                    (true, Some(model)) => ("The profile's".to_string(), format!("{model}, from reviewer.toml")),
                    _ => ("Claude's default".to_string(), default),
                };
                let mut choices = vec![Choice { label, about: default, set: Some(None), now: current.is_none(), dim: true, gap: false }];
                choices.extend(CLAUDE_MODELS.iter().map(|(name, about)| Choice::value(name, about, current.as_deref() == Some(*name))));
                choices.push(Choice { label: "Type a model…".to_string(), about: "a full id, as claude-opus-5-5".to_string(), set: None, now: false, dim: true, gap: false });
                let head = if reviewer { "Claude Code's model names; m sets review.model, which kebbie review runs".to_string() } else { format!("Claude Code's model names{built_in}") };
                (format!("{} · model", profile.name), head, choices)
            }
            (Field::Effort, agent) => {
                let model = agents.model(&profile);
                let cached = model.as_deref().and_then(|model| agents.cached(model)).filter(|_| agent == Agent::Codex);
                let shown = model.clone().unwrap_or_else(|| format!("{}'s default", agent.name()));
                let (levels, default, head): (Vec<String>, Option<String>, String) = match (agent, cached) {
                    (Agent::Codex, Some(cached)) => (cached.efforts.iter().filter(|level| profiles::CODEX_EFFORTS.contains(&level.as_str())).cloned().collect(), cached.default_effort.clone(), format!("the levels {} takes, from Codex's list", cached.name)),
                    (Agent::Codex, None) => (profiles::CODEX_EFFORTS.map(str::to_string).to_vec(), None, format!("{shown} is not in Codex's list, so these are every level Codex takes")),
                    (Agent::Claude, _) => (profiles::EFFORTS.map(str::to_string).to_vec(), None, "Claude Code's effort levels".to_string()),
                };
                let mut choices: Vec<Choice> = levels.iter().map(|level| {
                    let about = if default.as_deref() == Some(level.as_str()) { "Codex's default for this model" } else if level == "ultra" { "only some Codex models" } else { "" };
                    Choice::value(level, about, profile.effort.as_deref() == Some(level.as_str()))
                }).collect();
                let leave = match (agent, &default) {
                    (Agent::Codex, Some(level)) => format!("leave it to Codex: {level}"),
                    _ => format!("leave it to {}", agent.name()),
                };
                choices.push(Choice { label: "Default".to_string(), about: leave, set: Some(None), now: profile.effort.is_none(), dim: true, gap: true });
                (format!("{} · effort, for {shown}", profile.name), head, choices)
            }
        };
        let foot = if field == Field::Model { self.foot(&profile, field) } else { Vec::new() };
        let index = choices.iter().position(|choice| choice.now).unwrap_or(0);
        self.said = None;
        self.mode = Mode::Choose(Box::new(Chooser { field, title, head, choices, index, foot }));
        Effect::Handled
    }

    fn open_profile(&mut self) -> Effect {
        let Some(profile) = self.profile() else { return Effect::Handled };
        match profile.file {
            Some(file) => Effect::Open(file),
            None => {
                self.say(format!("{} is built in and has no file until a value is set: m or e writes profiles/{}.toml", profile.name, profile.name), MUTED);
                Effect::Handled
            }
        }
    }

    pub fn opened(&mut self, file: &Path, result: Result<bool>) {
        self.load();
        match result {
            Ok(_) => match self.agents.ignored.iter().find(|error| error.contains(&file.display().to_string())) {
                Some(error) => self.say(format!("✗ {}", error.replace(&file.display().to_string(), &self.shown_file(file))), RED),
                None => self.say(format!("{} loads", self.shown_file(file)), GREEN),
            },
            Err(error) => self.say(format!("✗ the editor: {error}"), RED),
        }
    }

    fn runs(&self, profile: &Profile) -> String {
        let model = match (&self.agents.own_model(profile), self.agents.default_model(profile.agent)) {
            (Some(model), _) => model.clone(),
            (None, Some(model)) => format!("{model} by {}'s default", profile.agent.name()),
            (None, None) => format!("{}'s default model", profile.agent.name()),
        };
        let effort = profile.effort.as_deref().map_or_else(String::new, |effort| format!(", effort {effort}"));
        format!("{} runs {} on {model}{effort}", profile.name, profile.agent.key())
    }

    fn write_profile(&mut self, changes: &[(&str, Option<&str>)]) {
        let Some(profile) = self.profile() else { return };
        let name = profile.name.clone();
        let edited = profiles::edit_in(&self.paths.profiles, &name, changes);
        self.load();
        let now = self.agents.profiles.iter().find(|profile| profile.name == name).map(|profile| self.runs(profile)).unwrap_or_default();
        let keys = changes.iter().map(|(key, _)| *key).collect::<Vec<_>>().join(if edited.as_ref().is_ok_and(|edited| *edited == Edited::Unchanged) { " or " } else { " and " });
        match edited {
            Ok(Edited::Wrote(file)) => {
                let said = match changes {
                    [(key, Some(value))] => format!("✓ wrote {key} = {} to {}", Set::Text(value.to_string()).shown(), self.shown_file(&file)),
                    _ => format!("✓ removed {keys} from {}", self.shown_file(&file)),
                };
                self.say(format!("{said} · {now}; a running task keeps what it started with"), GREEN);
            }
            Ok(Edited::Removed(file)) => self.say(format!("✓ removed {}, which held nothing else · {now}", self.shown_file(&file)), GREEN),
            Ok(Edited::Unchanged) if changes.len() > 1 => self.say(format!("{name} has no {keys} of its own · {now}"), MUTED),
            Ok(Edited::Unchanged) => self.say(format!("{name} has that {keys} already · {now}"), MUTED),
            Err(error) => self.say(format!("✗ {}", error.lines().next().unwrap_or_default().replace(&self.paths.profiles.display().to_string(), "profiles")), RED),
        }
    }

    fn layer_word(&self, layer: Layer) -> (String, Color) {
        match layer {
            Layer::Default => ("default".to_string(), MUTED),
            Layer::Machine => ("machine".to_string(), BLUE),
            Layer::Repo if self.linked() => ("team link".to_string(), PEACH),
            Layer::Repo => (format!("repo {}", self.repo_name()), TEAL),
        }
    }

    pub fn hint_lines(&self) -> (Line<'static>, Line<'static>) {
        let line = |hints: &[&str]| Line::from(Span::styled(hints.join(" · "), style(MUTED)));
        if self.on_profiles() {
            return (line(&["↑ ↓ profile", "m model", "e effort", "⏎ open file", "d default", "← → section", "esc board"]), line(&["↑ ↓ profile", "m model", "e effort", "⏎ open", "esc board"]));
        }
        let change = match self.current().map(|entry| entry.kind) {
            Some(Kind::Bool) => "space toggle",
            Some(Kind::Choice(_) | Kind::Skill) => "⏎ pick",
            _ => "⏎ edit",
        };
        (
            line(&["↑ ↓ key", "← → section", change, "d back to default", "tab machine / repo", "/ search", "esc board"]),
            line(&["↑ ↓ key", change, "d default", "tab repo", "esc board"]),
        )
    }

    fn row_lines(&self, width: usize) -> (Vec<Line<'static>>, usize) {
        let sections = self.sections();
        let Some((_, rows)) = sections.get(self.section) else {
            let empty = if self.filter_text().is_empty() { "no keys" } else { "no key matches" };
            return (vec![Line::from(Span::styled(empty.to_string(), style(MUTED)))], 0);
        };
        let key_width = rows.iter().map(|index| self.entries[*index].key.len()).max().unwrap_or(0).min(width / 2);
        let layer_width = 12usize;
        let value_width = width.saturating_sub(key_width + layer_width + 6);
        let (mut lines, mut selected) = (Vec::new(), 0);
        for (at, index) in rows.iter().enumerate() {
            let entry = &self.entries[*index];
            let chosen = at == self.row;
            let value = match (&self.mode, chosen) {
                (Mode::Edit { buffer, .. }, true) => Span::styled(format!("{buffer}▏"), bold(TEXT)),
                _ if entry.kind == Kind::Bool && entry.value == "true" => Span::styled("● on".to_string(), style(GREEN)),
                _ if entry.kind == Kind::Bool => Span::styled("○ off".to_string(), style(MUTED)),
                _ => Span::styled(cut(&entry.value, value_width), style(TEXT)),
            };
            let (layer, color) = self.layer_word(entry.layer);
            let used = text_width(&value.content);
            let bar = if chosen { Span::styled("▌", style(MAUVE)) } else { Span::raw(" ") };
            let key = Span::styled(format!(" {:<key_width$}  ", cut(entry.key, key_width)), if chosen { bold(TEXT) } else { style(TEXT) });
            let gap = Span::raw(" ".repeat(value_width.saturating_sub(used) + 1));
            let line = Line::from(vec![bar, key, value, gap, Span::styled(format!("{layer:>layer_width$}"), style(color))]);
            if chosen {
                selected = lines.len();
                lines.push(line.style(Style::new().bg(SURFACE)));
                lines.extend(wrap(&entry.about, width.saturating_sub(4), 8).into_iter().map(|text| Line::from(Span::styled(format!("    {text}"), style(SUBTEXT)))));
                lines.push(Line::default());
            } else {
                lines.push(line);
            }
        }
        (lines, selected)
    }

    fn profile_lines(&self, width: usize) -> (Vec<Line<'static>>, usize) {
        let agents = &self.agents;
        let name_width = agents.profiles.iter().map(|profile| profile.name.len()).max().unwrap_or(0).min(width / 4);
        let wheres: Vec<(String, Color)> = agents.profiles.iter().map(|profile| match (&profile.file, agents.review_model.as_ref().filter(|_| profile.name == profiles::REVIEWER)) {
            (_, Some((_, layer))) => (format!("from review.model · {}", self.layer_word(*layer).0), MUTED),
            (None, _) => ("built in".to_string(), MUTED),
            (Some(file), _) => (file.file_name().unwrap_or_default().to_string_lossy().to_string(), BLUE),
        }).collect();
        let where_width = wheres.iter().map(|(text, _)| text_width(text)).max().unwrap_or(0);
        let lead = 1 + name_width + 3 + 7;
        let model_width = width.saturating_sub(lead + 8 + where_width + 1).min(28);
        let (mut lines, mut selected) = (Vec::new(), 0);
        for (at, profile) in agents.profiles.iter().enumerate() {
            let chosen = at == self.row;
            let (agent, agent_color) = (profile.agent.key(), if profile.agent == Agent::Codex { BLUE } else { TEAL });
            let warned = agents.warning(profile).is_some();
            let model = match (&self.mode, chosen) {
                (Mode::Edit { typed: Typed::Model, buffer }, true) => Span::styled(format!("{buffer}▏"), bold(TEXT)),
                _ => {
                    let text = agents.model(profile).unwrap_or_else(|| "default".to_string());
                    let text = if warned { format!("{}  ⚠", cut(&text, model_width.saturating_sub(3))) } else { cut(&text, model_width) };
                    Span::styled(text, style(if warned { RED } else if agents.own_model(profile).is_some() { TEXT } else { MUTED }))
                }
            };
            let used = text_width(&model.content);
            let effort = profile.effort.clone().unwrap_or_else(|| "—".to_string());
            let (place, place_color) = wheres[at].clone();
            let fill = width.saturating_sub(lead + model_width.max(used) + 8).max(1);
            let line = Line::from(vec![
                if chosen { Span::styled("▌", style(MAUVE)) } else { Span::raw(" ") },
                Span::styled(format!(" {:<name_width$}  ", cut(&profile.name, name_width)), if chosen { bold(TEXT) } else { style(TEXT) }),
                Span::styled(format!("{agent:<7}"), style(agent_color)),
                model,
                Span::raw(" ".repeat(model_width.saturating_sub(used))),
                Span::styled(format!("{effort:<8}"), style(TEXT)),
                Span::styled(format!("{place:>fill$}"), style(place_color)),
            ]);
            if !chosen {
                lines.push(line);
                continue;
            }
            selected = lines.len();
            lines.push(line.style(Style::new().bg(SURFACE)));
            if !profile.about.is_empty() {
                for (number, text) in wrap(&profile.about, width.saturating_sub(12), 6).into_iter().enumerate() {
                    lines.push(Line::from(Span::styled(format!("{}{text}", if number == 0 { "    about: " } else { "           " }), style(SUBTEXT))));
                }
            }
            let mut keys = vec![Span::raw("    ")];
            for (number, (key, word)) in [("m", "model"), ("e", "effort"), ("⏎", "open the file"), ("d", "back to the default")].into_iter().enumerate() {
                keys.extend([Span::raw(if number == 0 { "" } else { "   " }), Span::styled(key, style(MAUVE)), Span::styled(format!(" {word}"), style(SUBTEXT))]);
            }
            lines.push(Line::from(keys));
            lines.push(Line::default());
        }
        for error in &agents.ignored {
            let error = error.replace(&self.paths.profiles.display().to_string(), "profiles");
            lines.extend(wrap(&format!("ignored: {error}"), width.saturating_sub(2), 3).into_iter().map(|text| Line::from(Span::styled(format!("  {text}"), style(RED)))));
        }
        lines.push(Line::default());
        lines.push(Line::from(Span::styled("  dim = the agent's own default:", style(MUTED))));
        let (claude, codex) = (home_relative(&self.paths.claude_home.join("settings.json")), home_relative(&self.paths.codex_home.join("config.toml")));
        lines.push(Line::from(Span::styled(format!("  Claude's from {claude}, Codex's from {codex}"), style(MUTED))));
        (lines, selected)
    }

    fn chooser_lines(chooser: &Chooser) -> Vec<Line<'static>> {
        let mut lines = vec![Line::from(Span::styled(chooser.head.clone(), style(MUTED))), Line::default()];
        for (at, choice) in chooser.choices.iter().enumerate() {
            if choice.gap {
                lines.push(Line::default());
            }
            let chosen = at == chooser.index;
            let label = if chosen { bold(MAUVE) } else if choice.dim { style(MUTED) } else { style(TEXT) };
            lines.push(Line::from(vec![
                Span::styled(if chosen { "▸ " } else { "  " }, style(MAUVE)),
                Span::styled(if choice.now { "● " } else { "  " }, style(GREEN)),
                Span::styled(format!("{:<18}", choice.label), label),
                Span::styled(choice.about.clone(), style(MUTED)),
            ]).style(if chosen { Style::new().bg(SURFACE) } else { Style::new() }));
        }
        lines.push(Line::default());
        if !chooser.foot.is_empty() {
            lines.extend(chooser.foot.iter().map(|text| Line::from(Span::styled(text.clone(), style(MUTED)))));
            lines.push(Line::default());
        }
        lines.push(Line::from(Span::styled("↑ ↓ choose · ⏎ set · esc close", style(FAINT))));
        lines
    }

    pub fn draw(&mut self, frame: &mut Frame, body: Rect) {
        let [title, tabs, _, main, said] = Layout::vertical([Constraint::Length(1), Constraint::Length(1), Constraint::Length(1), Constraint::Min(3), Constraint::Length(2)]).areas(body);
        let right = "from the board: , · or kebbie config edit";
        let gap = (title.width as usize).saturating_sub(8 + right.len()).max(1);
        frame.render_widget(Paragraph::new(Line::from(vec![Span::styled("Settings", bold(MAUVE)), Span::raw(" ".repeat(gap)), Span::styled(right, style(MUTED))])), title);
        let lit = |on: bool| if on { bold(MANTLE).bg(MAUVE) } else { style(MUTED) };
        let mut spans = vec![Span::styled(" This machine ", lit(self.tab == Tab::Machine)), Span::raw("  ")];
        if self.repo.is_some() {
            spans.push(Span::styled(format!(" Repo: {} ", self.repo_name()), lit(self.tab == Tab::Repo)));
            spans.push(Span::raw("  "));
        }
        let file = home_relative(&self.file());
        let profiles_file = home_relative(&self.paths.profiles.join("<name>.toml"));
        let writes = match (self.tab, self.linked()) {
            (Tab::Machine, _) if self.on_profiles() => format!("writes {profiles_file}"),
            (Tab::Machine, _) => format!("writes {file}"),
            (Tab::Repo, false) => format!("writes {file}, which wins over the machine"),
            (Tab::Repo, true) => format!("{file} links to a team file"),
        };
        spans.push(Span::styled(writes, style(MUTED)));
        frame.render_widget(Paragraph::new(Line::from(spans)), tabs);
        if let Some(error) = &self.error {
            let lines: Vec<Line> = wrap(error, main.width as usize, 6).into_iter().map(|text| Line::from(Span::styled(text, style(RED)))).collect();
            frame.render_widget(Paragraph::new(lines), main);
        } else {
            let [rail, _, rows] = Layout::horizontal([Constraint::Length(20), Constraint::Length(2), Constraint::Min(20)]).areas(main);
            let sections = self.sections();
            let rail_lines: Vec<Line> = sections.iter().enumerate().map(|(at, (name, keys))| {
                let chosen = at == self.section;
                let mark = if chosen { Span::styled("▸ ", style(MAUVE)) } else { Span::raw("  ") };
                let line = Line::from(vec![mark, Span::styled(cut(name, 14), if chosen { bold(TEXT) } else { style(TEXT) }), Span::styled(format!(" {}", keys.len()), style(FAINT))]);
                if chosen { line.style(Style::new().bg(SURFACE)) } else { line }
            }).collect();
            frame.render_widget(Paragraph::new(rail_lines), rail);
            let (lines, selected) = if self.on_profiles() { self.profile_lines(rows.width as usize) } else { self.row_lines(rows.width as usize) };
            let scroll = selected.saturating_sub(rows.height.saturating_sub(10) as usize);
            frame.render_widget(Paragraph::new(lines).scroll((scroll as u16, 0)), rows);
        }
        let under = match &self.mode {
            Mode::Filter { buffer } => Some((format!("/ {buffer}▏  · ⏎ keep · esc clear"), TEXT)),
            Mode::Edit { typed, .. } => Some((match typed { Typed::SkillName => "type a skill: <skill> or <plugin>:<skill>", Typed::SkillFile => "type the path of a .md file, starting with / or ~/", Typed::Model => "type the model's id, as the agent takes it", Typed::Value => "editing in place" }.to_string() + " · ⏎ save · esc cancel", SUBTEXT)),
            Mode::Normal if self.said.is_none() && self.on_profiles() => {
                let warned = self.profile().and_then(|profile| self.agents.warning(&profile)).or_else(|| self.agents.profiles.iter().find_map(|profile| self.agents.warning(profile)));
                warned.map(|text| (text, YELLOW))
            }
            _ => self.said.clone(),
        };
        if let Some((text, color)) = under {
            let lines: Vec<Line> = wrap(&text, said.width as usize, 2).into_iter().map(|text| Line::from(Span::styled(text, style(color)))).collect();
            frame.render_widget(Paragraph::new(lines), said);
        }
        match &self.mode {
            Mode::Pick { picks, index } => {
                let title = self.current().map_or_else(String::new, |entry| entry.key.to_string());
                let mut lines: Vec<Line> = picks.iter().enumerate().map(|(at, pick)| {
                    let chosen = at == *index;
                    let about = if pick.about.is_empty() { String::new() } else { format!("  · {}", pick.about) };
                    Line::from(vec![Span::raw(if chosen { "▸ " } else { "  " }), Span::styled(pick.label.clone(), if chosen { bold(MAUVE) } else { style(TEXT) }), Span::styled(about, style(MUTED))])
                        .style(if chosen { Style::new().bg(SURFACE) } else { Style::new() })
                }).collect();
                lines.push(Line::default());
                lines.push(Line::from(Span::styled("↑ ↓ choose · ⏎ set · esc close", style(FAINT))));
                modal(frame, title, lines, 72);
            }
            Mode::Choose(chooser) => {
                modal(frame, chooser.title.clone(), Self::chooser_lines(chooser), 72);
            }
            Mode::Team { key, .. } => {
                let target = std::fs::canonicalize(self.file()).map(|path| home_relative(&path)).unwrap_or_default();
                let lines = vec![
                    Line::from(Span::styled(format!("{} links to the team's file {target}.", home_relative(&self.file())), style(TEXT))),
                    Line::default(),
                    Line::from(vec![Span::styled("t", style(MAUVE)), Span::styled("  change the team's file, for everyone who links it", style(TEXT))]),
                    Line::from(vec![Span::styled("o", style(MAUVE)), Span::styled("  override it here: the link becomes a copy of this machine's own", style(TEXT))]),
                    Line::default(),
                    Line::from(Span::styled("esc cancel", style(FAINT))),
                ];
                modal(frame, format!("Change {key}"), lines, 76);
            }
            _ => {}
        }
    }

    fn draw_alone(&mut self, frame: &mut Frame) {
        let area = frame.area().inner(Margin { horizontal: 2, vertical: 1 });
        let [body, hint] = Layout::vertical([Constraint::Min(5), Constraint::Length(1)]).areas(area);
        self.draw(frame, body);
        let (full, short) = self.hint_lines();
        let line = if full.width() <= hint.width as usize { full } else { short };
        let spans: Vec<Span> = line.spans.into_iter().map(|span| Span::styled(span.content.replace("esc board", "esc quit"), span.style)).collect();
        frame.render_widget(Paragraph::new(Line::from(spans)), hint);
    }
}

pub fn open_in_editor(file: &Path) -> Result<bool> {
    std::process::Command::new("sh").args(["-c", "${VISUAL:-${EDITOR:-vi}} \"$1\"", "sh"]).arg(file).status().map(|status| status.success()).map_err(|error| error.to_string())
}

pub fn edit(cwd: &Path, repo: Option<&str>) -> Result<String> {
    use std::io::IsTerminal;
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        return Err("kebbie config edit needs a terminal; kebbie config prints every key with its value".to_string());
    }
    crate::permit::user_only("kebbie config edit")?;
    let repo = match repo {
        Some(path) => {
            let path = PathBuf::from(path);
            crate::repo::main_checkout(&path).map_err(|_| format!("--repo {}: not a git repository", path.display()))?;
            Some(path)
        }
        None => crate::repo::main_checkout(cwd).ok().map(|_| cwd.to_path_buf()),
    };
    let mut page = Page::new(Paths::current(), repo);
    page.load();
    let mut terminal = crate::board::terminal::init_terminal();
    let result = (|| -> Result<()> {
        loop {
            terminal.draw(|frame| page.draw_alone(frame)).map_err(|error| error.to_string())?;
            match ratatui::crossterm::event::read().map_err(|error| error.to_string())? {
                ratatui::crossterm::event::Event::Key(key) if key.kind == ratatui::crossterm::event::KeyEventKind::Press => {
                    if crate::keys::ctrl_c(&key) {
                        return Ok(());
                    }
                    match page.handle(key) {
                        Effect::Left => return Ok(()),
                        Effect::Open(file) => {
                            let status = crate::board::terminal::outside(&mut terminal, || open_in_editor(&file));
                            page.opened(&file, status);
                        }
                        Effect::Handled => {}
                    }
                }
                ratatui::crossterm::event::Event::Paste(text) => {
                    page.paste(&text);
                }
                _ => {}
            }
        }
    })();
    crate::board::terminal::restore_terminal();
    result.map(|()| page.said.map(|(text, _)| text).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        dir: PathBuf,
        page: Page,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn fixture(name: &str, machine: &str, repo_file: Option<&str>) -> Fixture {
        let dir = crate::sh::test_dir(name);
        let repo = dir.join("app");
        std::fs::create_dir_all(&repo).unwrap();
        crate::repo::git(&repo, &["init", "--quiet"]).unwrap();
        std::fs::create_dir_all(dir.join("repos")).unwrap();
        std::fs::write(dir.join("config.toml"), machine).unwrap();
        if let Some(text) = repo_file {
            std::fs::write(dir.join("repos/app.toml"), text).unwrap();
        }
        let kebbie = dir.join("kebbie");
        std::fs::create_dir_all(kebbie.join("skills/work-rules")).unwrap();
        std::fs::write(kebbie.join("skills/work-rules/SKILL.md"), "---\nname: work-rules\n---\n# Work rules\n").unwrap();
        let paths = Paths { machine: dir.join("config.toml"), repos_dir: dir.join("repos"), claude_home: dir.join("claude"), kebbie, profiles: dir.join("profiles"), codex_home: dir.join("codex") };
        let mut page = Page::new(paths, Some(repo));
        page.load();
        Fixture { page, dir }
    }

    fn press(page: &mut Page, keys: &[KeyCode]) {
        for code in keys {
            page.handle(KeyEvent::from(*code));
        }
    }

    fn type_text(page: &mut Page, text: &str) {
        for character in text.chars() {
            page.handle(KeyEvent::from(KeyCode::Char(character)));
        }
    }

    fn go_to(page: &mut Page, key: &str) {
        page.filter = String::new();
        let sections = page.sections();
        let (section, row) = sections.iter().enumerate().find_map(|(at, (_, rows))| rows.iter().position(|index| page.entries[*index].key == key).map(|row| (at, row))).unwrap_or_else(|| panic!("{key} is not on the {:?} tab", page.tab));
        (page.section, page.row) = (section, row);
    }

    fn said(page: &Page) -> String {
        page.said.clone().map(|(text, _)| text).unwrap_or_default()
    }

    fn file(fixture: &Fixture, name: &str) -> String {
        std::fs::read_to_string(fixture.dir.join(name)).unwrap_or_default()
    }

    fn screen(page: &mut Page) -> String {
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(140, 40)).unwrap();
        terminal.draw(|frame| page.draw_alone(frame)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        (0..buffer.area.height).map(|y| (0..buffer.area.width).map(|x| buffer[(x, y)].symbol().to_string()).collect::<String>().trim_end().to_string()).collect::<Vec<_>>().join("\n")
    }

    #[test]
    fn the_page_lists_what_kebbie_config_prints_with_each_layer() {
        let mut fixture = fixture("settings-list", "[task]\nsandbox = false\n", Some("[docs]\nrequired = true\n"));
        let loaded = config::resolve_at(fixture.dir.join("config.toml"), None).unwrap();
        let printed: Vec<&str> = config::entries(&loaded).iter().filter(|entry| entry.scope != Scope::Repo).map(|entry| entry.key).collect();
        let mut listed: Vec<&str> = fixture.page.entries.iter().map(|entry| entry.key).collect();
        let mut expected = printed.clone();
        listed.sort();
        expected.sort();
        assert_eq!(listed, expected, "the machine tab is kebbie config's keys, less the repo-only ones");
        let shown = screen(&mut fixture.page);
        assert!(shown.contains("Settings") && shown.contains(" This machine ") && shown.contains(" Repo: app "), "{shown}");
        assert!(shown.contains("▸ Tasks") && shown.contains("task.rules_skill") && shown.contains("\"kebbie:work-rules\""), "{shown}");
        assert!(shown.contains("task.sandbox") && shown.contains("○ off") && shown.lines().any(|line| line.contains("task.sandbox") && line.ends_with("machine")), "{shown}");
        assert!(shown.lines().any(|line| line.contains("task.rules_skill") && line.ends_with("default")), "{shown}");
        assert!(shown.contains("The skill with the rules for how work is planned"), "the selected key's text shows: {shown}");
        press(&mut fixture.page, &[KeyCode::Tab]);
        assert_eq!(fixture.page.tab, Tab::Repo);
        assert!(fixture.page.entries.iter().all(|entry| entry.scope != Scope::Machine), "a config.toml-only key never shows on the repo tab");
        assert!(fixture.page.entries.iter().any(|entry| entry.key == "docs.required"));
        go_to(&mut fixture.page, "docs.required");
        let shown = screen(&mut fixture.page);
        assert!(shown.lines().any(|line| line.contains("docs.required") && line.contains("● on") && line.ends_with("repo app")), "{shown}");
    }

    #[test]
    fn space_turns_steps_off_for_the_machine_and_on_again_for_one_repo() {
        let mut fixture = fixture("settings-steps", "", None);
        go_to(&mut fixture.page, "task.steps");
        assert!(screen(&mut fixture.page).lines().any(|line| line.contains("task.steps") && line.contains("● on")), "on by default");
        press(&mut fixture.page, &[KeyCode::Char(' ')]);
        assert_eq!(file(&fixture, "config.toml"), "[task]\nsteps = false\n");
        assert!(said(&fixture.page).starts_with("✓ wrote task.steps = false to config.toml"), "{}", said(&fixture.page));
        press(&mut fixture.page, &[KeyCode::Tab]);
        go_to(&mut fixture.page, "task.steps");
        press(&mut fixture.page, &[KeyCode::Char(' ')]);
        assert_eq!(file(&fixture, "repos/app.toml"), "[task]\nsteps = true\n", "a repo keeps its own way of working");
        assert_eq!(fixture.page.current().unwrap().layer, Layer::Repo);
    }

    #[test]
    fn space_toggles_a_boolean_in_the_file_of_the_tab_keeping_its_comments() {
        let mut fixture = fixture("settings-bool", "# mine\n[task]\nsandbox = true # build box\n", None);
        go_to(&mut fixture.page, "task.sandbox");
        press(&mut fixture.page, &[KeyCode::Char(' ')]);
        assert_eq!(file(&fixture, "config.toml"), "# mine\n[task]\nsandbox = false # build box\n");
        assert!(said(&fixture.page).starts_with("✓ wrote task.sandbox = false to config.toml · doctor: ok, config.toml parses"), "{}", said(&fixture.page));
        press(&mut fixture.page, &[KeyCode::Tab]);
        go_to(&mut fixture.page, "ci.watch");
        press(&mut fixture.page, &[KeyCode::Char(' ')]);
        assert_eq!(file(&fixture, "repos/app.toml"), "[ci]\nwatch = false\n", "the repo tab writes the repo's own file");
        assert!(!file(&fixture, "config.toml").contains("[ci]"));
        assert_eq!(fixture.page.current().unwrap().layer, Layer::Repo);
    }

    #[test]
    fn a_choice_is_picked_from_its_list_and_none_removes_the_key() {
        let mut fixture = fixture("settings-choice", "", None);
        go_to(&mut fixture.page, "task.also_noticed");
        press(&mut fixture.page, &[KeyCode::Enter]);
        let shown = screen(&mut fixture.page);
        assert!(shown.contains("▸ \"backlog\"  · now") && shown.contains("\"ask\""), "{shown}");
        press(&mut fixture.page, &[KeyCode::Down, KeyCode::Down, KeyCode::Enter]);
        assert_eq!(file(&fixture, "config.toml"), "[task]\nalso_noticed = \"ask\"\n");
        go_to(&mut fixture.page, "task.permission_mode");
        press(&mut fixture.page, &[KeyCode::Enter, KeyCode::Down, KeyCode::Down, KeyCode::Enter]);
        assert!(file(&fixture, "config.toml").contains("permission_mode = \"acceptEdits\""));
        press(&mut fixture.page, &[KeyCode::Enter]);
        assert!(matches!(fixture.page.mode, Mode::Pick { index: 2, .. }), "the pick opens on the value in effect");
        press(&mut fixture.page, &[KeyCode::Up, KeyCode::Up, KeyCode::Enter]);
        assert!(!file(&fixture, "config.toml").contains("permission_mode"), "none leaves it to Claude: the key goes");
    }

    #[test]
    fn text_numbers_and_lists_are_edited_in_place_and_a_bad_value_writes_nothing() {
        let mut fixture = fixture("settings-text", "[restart]\nidle_minutes = 10\n", None);
        go_to(&mut fixture.page, "restart.idle_minutes");
        press(&mut fixture.page, &[KeyCode::Enter, KeyCode::Backspace, KeyCode::Backspace]);
        type_text(&mut fixture.page, "25");
        press(&mut fixture.page, &[KeyCode::Enter]);
        assert_eq!(file(&fixture, "config.toml"), "[restart]\nidle_minutes = 25\n");
        press(&mut fixture.page, &[KeyCode::Enter, KeyCode::Backspace, KeyCode::Backspace]);
        type_text(&mut fixture.page, "0");
        press(&mut fixture.page, &[KeyCode::Enter]);
        assert!(said(&fixture.page).starts_with("✗ ") && said(&fixture.page).contains("1 to 1440"), "{}", said(&fixture.page));
        assert_eq!(file(&fixture, "config.toml"), "[restart]\nidle_minutes = 25\n", "a value kebbie refuses is never written");
        press(&mut fixture.page, &[KeyCode::Enter]);
        type_text(&mut fixture.page, "x");
        press(&mut fixture.page, &[KeyCode::Enter]);
        assert!(said(&fixture.page).contains("\"25x\" is not a whole number"), "{}", said(&fixture.page));
        go_to(&mut fixture.page, "task.test_commands");
        press(&mut fixture.page, &[KeyCode::Enter]);
        type_text(&mut fixture.page, "bin/test, make check");
        press(&mut fixture.page, &[KeyCode::Enter]);
        assert!(file(&fixture, "config.toml").contains("[task]\ntest_commands = [\"bin/test\", \"make check\"]\n"), "{}", file(&fixture, "config.toml"));
        go_to(&mut fixture.page, "machine.name");
        press(&mut fixture.page, &[KeyCode::Enter, KeyCode::Char('u')]);
        fixture.page.handle(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        type_text(&mut fixture.page, "devbox");
        press(&mut fixture.page, &[KeyCode::Enter]);
        assert!(file(&fixture, "config.toml").contains("[machine]\nname = \"devbox\"\n"));
        press(&mut fixture.page, &[KeyCode::Enter]);
        type_text(&mut fixture.page, "zzz");
        press(&mut fixture.page, &[KeyCode::Esc]);
        assert!(file(&fixture, "config.toml").contains("name = \"devbox\""), "esc cancels the edit");
    }

    #[test]
    fn d_removes_the_key_so_the_layer_below_applies() {
        let mut fixture = fixture("settings-remove", "[ci]\nwatch = false\n", Some("# team\n[ci]\nwatch = true\n"));
        press(&mut fixture.page, &[KeyCode::Tab]);
        go_to(&mut fixture.page, "ci.watch");
        assert_eq!(fixture.page.current().unwrap().layer, Layer::Repo);
        press(&mut fixture.page, &[KeyCode::Char('d')]);
        assert_eq!(file(&fixture, "repos/app.toml"), "# team\n[ci]\n");
        let entry = fixture.page.current().unwrap();
        assert_eq!((entry.value.as_str(), entry.layer), ("false", Layer::Machine), "the machine's value applies once the repo's goes");
        assert!(said(&fixture.page).starts_with("✓ removed ci.watch from repos/app.toml"), "{}", said(&fixture.page));
    }

    #[test]
    fn a_value_from_a_team_link_asks_whether_to_change_the_team_file_or_override_it_here() {
        let mut fixture = fixture("settings-team", "", None);
        let team = fixture.dir.join("team.toml");
        std::fs::write(&team, "[ci]\nwatch = true\n").unwrap();
        std::os::unix::fs::symlink(&team, fixture.dir.join("repos/app.toml")).unwrap();
        fixture.page.load();
        press(&mut fixture.page, &[KeyCode::Tab]);
        go_to(&mut fixture.page, "ci.watch");
        assert!(screen(&mut fixture.page).lines().any(|line| line.contains("ci.watch") && line.ends_with("team link")));
        press(&mut fixture.page, &[KeyCode::Char(' ')]);
        let shown = screen(&mut fixture.page);
        assert!(shown.contains("change the team's file") && shown.contains("override it here"), "{shown}");
        assert_eq!(std::fs::read_to_string(&team).unwrap(), "[ci]\nwatch = true\n", "nothing is written before the answer");
        press(&mut fixture.page, &[KeyCode::Char('t')]);
        assert_eq!(std::fs::read_to_string(&team).unwrap(), "[ci]\nwatch = false\n");
        press(&mut fixture.page, &[KeyCode::Char(' '), KeyCode::Char('o')]);
        let own = fixture.dir.join("repos/app.toml");
        assert!(!std::fs::symlink_metadata(&own).unwrap().file_type().is_symlink());
        assert_eq!((std::fs::read_to_string(&own).unwrap(), std::fs::read_to_string(&team).unwrap()), ("[ci]\nwatch = true\n".to_string(), "[ci]\nwatch = false\n".to_string()));
    }

    #[test]
    fn a_skill_key_offers_built_in_a_name_a_file_or_a_copy_of_kebbies_to_edit() {
        let mut fixture = fixture("settings-skill", "", None);
        go_to(&mut fixture.page, "task.rules_skill");
        press(&mut fixture.page, &[KeyCode::Enter]);
        let shown = screen(&mut fixture.page);
        assert!(shown.contains("\"kebbie:work-rules\"  · built in") && shown.contains("Your skill, by name") && shown.contains("A Markdown file, by path") && shown.contains("Copy kebbie's to"), "{shown}");
        press(&mut fixture.page, &[KeyCode::Down, KeyCode::Down, KeyCode::Enter]);
        type_text(&mut fixture.page, "~/code/team/RULES.md");
        press(&mut fixture.page, &[KeyCode::Enter]);
        assert_eq!(file(&fixture, "config.toml"), "[task]\nrules_skill = \"~/code/team/RULES.md\"\n");
        press(&mut fixture.page, &[KeyCode::Enter, KeyCode::Down, KeyCode::Enter]);
        type_text(&mut fixture.page, "team-rules");
        press(&mut fixture.page, &[KeyCode::Enter]);
        assert_eq!(file(&fixture, "config.toml"), "[task]\nrules_skill = \"team-rules\"\n");
        press(&mut fixture.page, &[KeyCode::Enter, KeyCode::Down, KeyCode::Down, KeyCode::Down, KeyCode::Enter]);
        let copied = fixture.dir.join("claude/skills/work-rules/SKILL.md");
        assert_eq!(std::fs::read_to_string(&copied).unwrap(), "---\nname: work-rules\n---\n# Work rules\n");
        assert_eq!(file(&fixture, "config.toml"), "[task]\nrules_skill = \"work-rules\"\n");
        assert!(said(&fixture.page).contains("copied kebbie's skill to") && said(&fixture.page).contains("work-rules/SKILL.md"), "{}", said(&fixture.page));
        std::fs::write(&copied, "mine").unwrap();
        press(&mut fixture.page, &[KeyCode::Enter, KeyCode::Down, KeyCode::Down, KeyCode::Down, KeyCode::Enter]);
        assert_eq!(std::fs::read_to_string(&copied).unwrap(), "mine", "a copy already there is the user's and stays");
        press(&mut fixture.page, &[KeyCode::Enter, KeyCode::Enter]);
        assert_eq!(file(&fixture, "config.toml"), "[task]\nrules_skill = \"kebbie:work-rules\"\n");
        press(&mut fixture.page, &[KeyCode::Enter, KeyCode::Down, KeyCode::Down, KeyCode::Enter]);
        type_text(&mut fixture.page, "RULES.md");
        press(&mut fixture.page, &[KeyCode::Enter]);
        assert!(said(&fixture.page).contains("a file goes by its absolute or ~ path"), "{}", said(&fixture.page));
        press(&mut fixture.page, &[KeyCode::Enter, KeyCode::Down, KeyCode::Enter]);
        type_text(&mut fixture.page, "team/rules");
        press(&mut fixture.page, &[KeyCode::Enter]);
        assert!(said(&fixture.page).contains("not a skill name or a Markdown file"), "{}", said(&fixture.page));
        assert_eq!(file(&fixture, "config.toml"), "[task]\nrules_skill = \"kebbie:work-rules\"\n");
    }

    #[test]
    fn slash_filters_the_keys_tab_switches_and_esc_leaves() {
        let mut fixture = fixture("settings-filter", "", None);
        press(&mut fixture.page, &[KeyCode::Char('/')]);
        type_text(&mut fixture.page, "idle_m");
        press(&mut fixture.page, &[KeyCode::Enter]);
        assert_eq!(fixture.page.current().map(|entry| entry.key), Some("restart.idle_minutes"));
        assert_eq!(fixture.page.sections()[0].1.len(), 1);
        assert!(screen(&mut fixture.page).contains("Search: idle_m"));
        assert!(matches!(fixture.page.handle(KeyEvent::from(KeyCode::Esc)), Effect::Handled), "the first esc clears the search");
        assert!(fixture.page.filter.is_empty());
        assert!(matches!(fixture.page.handle(KeyEvent::from(KeyCode::Esc)), Effect::Left));
        let mut alone = Page::new(Paths { machine: fixture.dir.join("config.toml"), repos_dir: fixture.dir.join("repos"), claude_home: fixture.dir.join("claude"), kebbie: fixture.dir.join("kebbie"), profiles: fixture.dir.join("profiles"), codex_home: fixture.dir.join("codex") }, None);
        alone.load();
        press(&mut alone, &[KeyCode::Tab]);
        assert_eq!(alone.tab, Tab::Machine);
        assert!(said(&alone).contains("No repo here"));
    }

    #[test]
    fn a_file_that_does_not_parse_is_shown_and_left_alone() {
        let mut fixture = fixture("settings-broken", "[task]\nsandbox = \"yes\"\n", None);
        assert!(fixture.page.entries.is_empty());
        assert!(screen(&mut fixture.page).contains("task.sandbox: must be true or false"));
        press(&mut fixture.page, &[KeyCode::Char(' '), KeyCode::Char('d')]);
        assert_eq!(file(&fixture, "config.toml"), "[task]\nsandbox = \"yes\"\n");
    }

    const CODEX_BUILD: &str = "# for small builds\nabout = \"Codex, building: a small, well-defined task done by another model family\"\nagent = \"codex\" # not Claude\nmodel = \"gpt-6.1-sol\"\neffort = \"medium\"\n";

    fn agents_fixture(name: &str) -> Fixture {
        let fixture = fixture(name, "", None);
        let dir = &fixture.dir;
        for folder in ["profiles", "codex", "claude"] {
            std::fs::create_dir_all(dir.join(folder)).unwrap();
        }
        std::fs::write(dir.join("profiles/codex.toml"), "about = \"Codex on its own config\"\nagent = \"codex\"\neffort = \"medium\"\n").unwrap();
        std::fs::write(dir.join("profiles/codex-build.toml"), CODEX_BUILD).unwrap();
        std::fs::write(dir.join("profiles/reviewer.toml"), "model = \"opus\"\neffort = \"max\"\n").unwrap();
        std::fs::write(dir.join("codex/config.toml"), "model = \"gpt-5.1-codex-max\"\n").unwrap();
        std::fs::write(dir.join("codex/models_cache.json"), crate::agent::codex::CACHE_FIXTURE).unwrap();
        std::fs::write(dir.join("claude/settings.json"), "{\"model\": \"opus\"}").unwrap();
        let mut fixture = fixture;
        fixture.page.load();
        fixture
    }

    fn go_to_profile(page: &mut Page, name: &str) {
        page.filter = String::new();
        page.section = page.sections().iter().position(|(section, _)| section == PROFILES).expect("a Profiles section");
        page.row = page.agents.profiles.iter().position(|profile| profile.name == name).unwrap_or_else(|| panic!("no profile {name}"));
    }

    fn screen_at(page: &mut Page, width: u16, height: u16) -> (String, ratatui::buffer::Buffer) {
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| page.draw_alone(frame)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let text = (0..buffer.area.height).map(|y| (0..buffer.area.width).map(|x| buffer[(x, y)].symbol().to_string()).collect::<String>().trim_end().to_string()).collect::<Vec<_>>().join("\n");
        (text, buffer)
    }

    fn color_of(buffer: &ratatui::buffer::Buffer, line: &str, word: &str) -> Option<Color> {
        let lines: Vec<String> = (0..buffer.area.height).map(|y| (0..buffer.area.width).map(|x| buffer[(x, y)].symbol().to_string()).collect()).collect();
        let y = lines.iter().position(|text| text.contains(line))?;
        let x = lines[y].chars().collect::<Vec<_>>().windows(word.chars().count()).position(|window| window.iter().collect::<String>() == word)?;
        buffer[(x as u16, y as u16)].fg.into()
    }

    #[test]
    fn the_profiles_section_lists_each_profile_with_its_agent_model_effort_and_file() {
        let mut fixture = agents_fixture("settings-profiles");
        let names: Vec<String> = fixture.page.sections().into_iter().map(|(name, _)| name).collect();
        let at = names.iter().position(|name| name == PROFILES).unwrap();
        assert_eq!((names[at - 1].as_str(), names[at + 1].as_str()), ("Review & docs", "Updates"));
        go_to_profile(&mut fixture.page, "codex-build");
        let (shown, buffer) = screen_at(&mut fixture.page, 100, 30);
        assert!(shown.contains("▸ Profiles 5"), "{shown}");
        assert!(screen_at(&mut fixture.page, 300, 30).0.contains(&format!("writes {}", fixture.dir.join("profiles/<name>.toml").display())));
        let row = |name: &str| shown.lines().find(|line| line.contains(&format!(" {name} "))).unwrap_or_else(|| panic!("{name}: {shown}")).to_string();
        assert!(row("task").contains("claude opus") && row("task").contains("—") && row("task").ends_with("built in"), "{}", row("task"));
        assert!(row("reviewer").contains("opus") && row("reviewer").contains("max") && row("reviewer").ends_with(" reviewer.toml"), "{}", row("reviewer"));
        assert!(row("codex").contains("codex  gpt-5.1-codex-max  ⚠") && row("codex").contains("medium") && row("codex").ends_with("codex.toml"), "{}", row("codex"));
        assert!(row("codex-build").contains("▌ codex-build  codex  gpt-6.1-sol") && row("codex-build").ends_with("codex-build.toml"), "{}", row("codex-build"));
        assert!(shown.contains("about: Codex, building: a small, well-defined task done by another") && shown.contains("m model   e effort   ⏎ open the file   d back to the default"), "{shown}");
        assert!(shown.contains("dim = the agent's own default:") && shown.contains("Claude's from "), "{shown}");
        assert!(screen_at(&mut fixture.page, 400, 30).0.contains(&format!("Claude's from {}, Codex's from {}", fixture.dir.join("claude/settings.json").display(), fixture.dir.join("codex/config.toml").display())));
        assert!(shown.contains("⚠ codex runs gpt-5.1-codex-max from Codex's config.toml, which Codex does not offer") && shown.contains("account; pick a model for it with m."), "{shown}");
        assert!(shown.lines().any(|line| line.trim_start().starts_with("↑ ↓ profile · m model · e effort · ⏎ open file · d default · ← → section · esc quit")), "{shown}");
        assert_eq!(color_of(&buffer, " task ", "opus"), Some(MUTED), "the agent's own default shows dim");
        assert_eq!(color_of(&buffer, " reviewer ", "opus"), Some(TEXT), "the profile's own model does not");
        assert_eq!(color_of(&buffer, " codex  ", "gpt-5.1-codex-max"), Some(RED));
        press(&mut fixture.page, &[KeyCode::Tab]);
        assert!(!fixture.page.sections().iter().any(|(name, _)| name == PROFILES), "profiles are this machine's, never a repo's");
    }

    #[test]
    fn m_on_a_codex_profile_picks_from_codexs_own_list_and_writes_model_alone() {
        let mut fixture = agents_fixture("settings-codex-model");
        go_to_profile(&mut fixture.page, "codex-build");
        press(&mut fixture.page, &[KeyCode::Char('m')]);
        let (shown, _) = screen_at(&mut fixture.page, 100, 30);
        assert!(shown.contains("╭ codex-build · model ") && shown.contains("Codex's own list for your account"), "{shown}");
        assert!(matches!(&fixture.page.mode, Mode::Choose(chooser) if chooser.head.ends_with("codex/models_cache.json)")));
        assert!(shown.contains("▸ ● GPT-6.1-Sol       gpt-6.1-sol") && shown.contains("    GPT-6-Astra       gpt-6-astra") && shown.contains("    GPT-6.1 Terra     gpt-6.1-terra"), "{shown}");
        assert!(shown.contains("    Codex's default   gpt-5.1-codex-max, not offered to you ⚠") && shown.contains("    Type a model…     any id Codex accepts"), "{shown}");
        assert!(shown.contains("writes model = \"…\" in codex-build.toml, the rest stays as it is") && shown.contains("a task already running keeps its model; the next start takes it"), "{shown}");
        assert!(matches!(&fixture.page.mode, Mode::Choose(chooser) if !chooser.choices.iter().any(|choice| choice.about == "gpt-5.1-codex-max")), "a model Codex hides is not offered, only named");
        press(&mut fixture.page, &[KeyCode::Down, KeyCode::Enter]);
        assert_eq!(file(&fixture, "profiles/codex-build.toml"), CODEX_BUILD.replace("gpt-6.1-sol", "gpt-6-astra"), "the one line changes, comments and all else stay");
        assert!(said(&fixture.page).starts_with("✓ wrote model = \"gpt-6-astra\" to profiles/codex-build.toml · codex-build runs codex on gpt-6-astra, effort medium"), "{}", said(&fixture.page));
        press(&mut fixture.page, &[KeyCode::Char('m'), KeyCode::Down, KeyCode::Down, KeyCode::Down, KeyCode::Enter]);
        assert!(matches!(fixture.page.mode, Mode::Edit { typed: Typed::Model, .. }));
        fixture.page.handle(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        type_text(&mut fixture.page, "gpt-7-preview");
        press(&mut fixture.page, &[KeyCode::Enter]);
        assert!(file(&fixture, "profiles/codex-build.toml").contains("\nmodel = \"gpt-7-preview\"\n"), "any id is taken");
        assert!(screen_at(&mut fixture.page, 100, 30).0.contains("gpt-7-preview  ⚠"), "an id Codex does not list is warned about");
        press(&mut fixture.page, &[KeyCode::Char('m'), KeyCode::Down, KeyCode::Down, KeyCode::Down, KeyCode::Enter]);
        assert!(!file(&fixture, "profiles/codex-build.toml").contains("model ="), "Codex's default removes the key");
        assert!(said(&fixture.page).starts_with("✓ removed model from profiles/codex-build.toml · codex-build runs codex on gpt-5.1-codex-max by Codex's default"), "{}", said(&fixture.page));
        std::fs::remove_file(fixture.dir.join("codex/models_cache.json")).unwrap();
        fixture.page.load();
        press(&mut fixture.page, &[KeyCode::Char('m')]);
        let (shown, _) = screen_at(&mut fixture.page, 100, 30);
        assert!(shown.contains("no list from Codex at ") && shown.contains("Codex's default   gpt-5.1-codex-max") && shown.contains("Type a model…") && !shown.contains("GPT-6"), "{shown}");
        assert!(!shown.contains("not offered"), "without Codex's list nothing is called not offered: {shown}");
    }

    #[test]
    fn e_offers_the_levels_codex_lists_for_the_profiles_model_ultra_among_them() {
        let mut fixture = agents_fixture("settings-codex-effort");
        go_to_profile(&mut fixture.page, "codex-build");
        press(&mut fixture.page, &[KeyCode::Char('e')]);
        let (shown, _) = screen_at(&mut fixture.page, 100, 30);
        assert!(shown.contains("╭ codex-build · effort, for gpt-6.1-sol ") && shown.contains("the levels GPT-6.1-Sol takes, from Codex's list"), "{shown}");
        assert!(shown.contains("    low               Codex's default for this model") && shown.contains("▸ ● medium") && shown.contains("    ultra             only some Codex models"), "{shown}");
        assert!(shown.contains("    Default           leave it to Codex: low"), "{shown}");
        press(&mut fixture.page, &[KeyCode::Down, KeyCode::Down, KeyCode::Down, KeyCode::Down, KeyCode::Enter]);
        assert_eq!(file(&fixture, "profiles/codex-build.toml"), CODEX_BUILD.replace("effort = \"medium\"", "effort = \"ultra\""));
        assert_eq!(fixture.page.profile().unwrap().effort.as_deref(), Some("ultra"));
        go_to_profile(&mut fixture.page, "codex");
        press(&mut fixture.page, &[KeyCode::Char('e')]);
        let (shown, _) = screen_at(&mut fixture.page, 100, 30);
        assert!(shown.contains("codex · effort, for gpt-5.1-codex-max") && shown.contains("the levels gpt-5.1-codex-max takes"), "a hidden model's own levels: {shown}");
        press(&mut fixture.page, &[KeyCode::Down, KeyCode::Enter]);
        assert!(!file(&fixture, "profiles/codex.toml").contains("effort"), "Default leaves it to Codex");
    }

    #[test]
    fn m_on_a_built_in_claude_profile_writes_its_own_file_and_d_takes_it_away_again() {
        let mut fixture = agents_fixture("settings-claude-model");
        go_to_profile(&mut fixture.page, "task");
        assert!(matches!(fixture.page.handle(KeyEvent::from(KeyCode::Enter)), Effect::Handled));
        assert!(said(&fixture.page).contains("task is built in and has no file until a value is set"), "{}", said(&fixture.page));
        press(&mut fixture.page, &[KeyCode::Char('m')]);
        let (shown, _) = screen_at(&mut fixture.page, 100, 30);
        assert!(shown.contains("╭ task · model ") && shown.contains("Claude Code's model names; task is built in"), "{shown}");
        assert!(matches!(&fixture.page.mode, Mode::Choose(chooser) if chooser.choices[0].about == format!("opus, from {}", fixture.dir.join("claude/settings.json").display())));
        assert!(shown.contains("▸ ● Claude's default  opus, from") && shown.contains("    opus              the newest Opus") && shown.contains("    fable             the newest Fable") && shown.contains("    Type a model…     a full id, as claude-opus-5-5"), "{shown}");
        assert!(shown.contains("task is built in: setting a model writes profiles/task.toml with") && shown.contains("model alone, and the built-in's role and rules stay as they are.") && shown.contains("d on the row removes that file again.") && shown.contains("a task already running keeps its model; the next start takes it"), "{shown}");
        press(&mut fixture.page, &[KeyCode::Down, KeyCode::Down, KeyCode::Enter]);
        assert_eq!(file(&fixture, "profiles/task.toml"), "model = \"sonnet\"\n");
        assert!(screen_at(&mut fixture.page, 100, 30).0.lines().any(|line| line.contains(" task ") && line.contains("sonnet") && line.ends_with(" task.toml")));
        press(&mut fixture.page, &[KeyCode::Char('e')]);
        let (shown, _) = screen_at(&mut fixture.page, 100, 30);
        assert!(shown.contains("task · effort, for sonnet") && shown.contains("Claude Code's effort levels") && !shown.contains("ultra") && shown.contains("leave it to Claude"), "{shown}");
        press(&mut fixture.page, &[KeyCode::Up, KeyCode::Enter]);
        assert_eq!(file(&fixture, "profiles/task.toml"), "model = \"sonnet\"\neffort = \"max\"\n");
        assert!(matches!(fixture.page.handle(KeyEvent::from(KeyCode::Enter)), Effect::Open(file) if file == fixture.dir.join("profiles/task.toml")), "⏎ opens the file once there is one");
        press(&mut fixture.page, &[KeyCode::Char('m'), KeyCode::Down, KeyCode::Down, KeyCode::Down, KeyCode::Down, KeyCode::Down, KeyCode::Enter]);
        fixture.page.handle(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        press(&mut fixture.page, &[KeyCode::Enter]);
        assert!(said(&fixture.page).contains("✗ profiles/task.toml: model: must not be empty"), "{}", said(&fixture.page));
        assert_eq!(file(&fixture, "profiles/task.toml"), "model = \"sonnet\"\neffort = \"max\"\n", "a value the parser refuses is never written");
        press(&mut fixture.page, &[KeyCode::Char('d')]);
        assert!(!fixture.dir.join("profiles/task.toml").exists());
        assert!(said(&fixture.page).starts_with("✓ removed profiles/task.toml, which held nothing else · task runs claude on opus by Claude's default"), "{}", said(&fixture.page));
        press(&mut fixture.page, &[KeyCode::Char('d')]);
        assert!(said(&fixture.page).starts_with("task has no model or effort of its own"), "{}", said(&fixture.page));
        go_to_profile(&mut fixture.page, "codex-build");
        press(&mut fixture.page, &[KeyCode::Char('d')]);
        assert_eq!(file(&fixture, "profiles/codex-build.toml"), "# for small builds\nabout = \"Codex, building: a small, well-defined task done by another model family\"\nagent = \"codex\" # not Claude\n", "a file profile keeps everything but the two keys");
    }

    #[test]
    fn a_profile_file_that_does_not_load_is_named_under_the_list() {
        let mut fixture = agents_fixture("settings-profile-broken");
        std::fs::write(fixture.dir.join("profiles/broken.toml"), "agent = \"gemini\"\n").unwrap();
        fixture.page.load();
        go_to_profile(&mut fixture.page, "task");
        let (shown, _) = screen_at(&mut fixture.page, 140, 40);
        assert!(shown.contains("ignored: profiles/broken.toml: agent: \"gemini\" is not an agent kebbie runs"), "{shown}");
        fixture.page.opened(&fixture.dir.join("profiles/broken.toml"), Ok(true));
        assert!(said(&fixture.page).starts_with("✗ profiles/broken.toml: agent:"), "{}", said(&fixture.page));
        fixture.page.opened(&fixture.dir.join("profiles/codex.toml"), Ok(true));
        assert_eq!(said(&fixture.page), "profiles/codex.toml loads");
    }

    #[test]
    fn the_reviewer_row_shows_review_model_and_m_changes_review_model_never_reviewer_toml() {
        let mut fixture = agents_fixture("settings-review-model");
        std::fs::write(fixture.dir.join("config.toml"), "# mine\n[review]\nmodel = \"sonnet\"\n").unwrap();
        fixture.page.load();
        go_to_profile(&mut fixture.page, "reviewer");
        let (shown, _) = screen_at(&mut fixture.page, 100, 30);
        let row = shown.lines().find(|line| line.contains(" reviewer ")).unwrap().to_string();
        assert!(row.contains("claude sonnet") && row.contains("max") && row.ends_with("from review.model · machine"), "the model kebbie review runs, and where it comes from: {row}");
        press(&mut fixture.page, &[KeyCode::Char('m')]);
        let (shown, _) = screen_at(&mut fixture.page, 100, 30);
        assert!(shown.contains("Claude Code's model names; m sets review.model") && shown.contains("    The profile's     opus, from reviewer.toml") && shown.contains("▸ ● sonnet"), "{shown}");
        assert!(shown.contains("writes review.model = \"…\" in "), "{shown}");
        press(&mut fixture.page, &[KeyCode::Down, KeyCode::Enter]);
        assert_eq!(file(&fixture, "config.toml"), "# mine\n[review]\nmodel = \"haiku\"\n");
        assert_eq!(file(&fixture, "profiles/reviewer.toml"), "model = \"opus\"\neffort = \"max\"\n", "the reviewer's model has one place");
        assert!(said(&fixture.page).starts_with("✓ wrote review.model = \"haiku\" to config.toml"), "{}", said(&fixture.page));
        press(&mut fixture.page, &[KeyCode::Char('m'), KeyCode::Up, KeyCode::Up, KeyCode::Up, KeyCode::Enter]);
        assert!(!file(&fixture, "config.toml").contains("model ="), "the default removes review.model: {}", file(&fixture, "config.toml"));
        let (shown, _) = screen_at(&mut fixture.page, 100, 30);
        assert!(shown.lines().any(|line| line.contains(" reviewer ") && line.contains("claude opus") && line.ends_with(" reviewer.toml")), "{shown}");
        press(&mut fixture.page, &[KeyCode::Char('e'), KeyCode::Up, KeyCode::Enter]);
        assert_eq!(file(&fixture, "profiles/reviewer.toml"), "model = \"opus\"\neffort = \"xhigh\"\n", "e keeps editing the profile's effort");
        assert!(!file(&fixture, "config.toml").contains("effort"));
        std::fs::write(fixture.dir.join("config.toml"), "[review]\nmodel = \"sonnet\"\n").unwrap();
        fixture.page.load();
        press(&mut fixture.page, &[KeyCode::Char('d')]);
        assert_eq!((file(&fixture, "config.toml"), fixture.dir.join("profiles/reviewer.toml").exists()), ("[review]\n".to_string(), false), "d takes the reviewer back to its defaults, review.model with them");
        assert!(said(&fixture.page).starts_with("✓ removed review.model from config.toml") && said(&fixture.page).contains("✓ removed profiles/reviewer.toml"), "{}", said(&fixture.page));
    }
}
