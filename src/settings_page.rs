use std::path::{Path, PathBuf};

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Constraint, Layout, Margin, Position, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::agent::Agent;
use crate::agent::codex::CachedModel;
use crate::board::widgets::{self, Answer, BLUE, FAINT, GREEN, MANTLE, MAUVE, MUTED, PEACH, RED, SUBTEXT, SURFACE, Spot, TEAL, TEXT, YELLOW, answer_lines, bold, cut, group_line, hint_line, modal, modal_spots, options_lines, style, text_width, wrap};
use crate::config::{self, Entry, Kind, Layer, Scope};
use crate::config_edit::{self, Set};
use crate::profile_wizard::{self, Pick, Wizard};
use crate::profiles::{self, Edited, Profile, Role};
use crate::sh::Result;

pub enum Effect {
    Handled,
    Left,
    Open(PathBuf),
    Show(PathBuf),
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
enum Then {
    Write(Change),
    Type(Typed),
    Field(Field, Option<String>),
    TypeModel,
    Role(Role),
}

type Choice = Answer<Then>;

fn choice(label: &str, about: &str, then: Then, now: bool) -> Choice {
    Answer::new(label, about, then).now(now)
}

// A value in effect that no answer offers becomes one, before the last `behind` answers, so ⏎ keeps it rather than writing the first.
fn keep_current(choices: &mut Vec<Choice>, behind: usize, current: Option<&str>, make: impl FnOnce(&str) -> Choice) {
    if let Some(current) = current && !choices.iter().any(|choice| choice.now) {
        choices.insert(choices.len().saturating_sub(behind), make(current).now(true));
    }
}

struct Chooser {
    title: String,
    question: String,
    explain: String,
    choices: Vec<Choice>,
    index: usize,
    foot: Vec<String>,
    opens: bool,
}

impl Chooser {
    fn new(title: String, question: &str, explain: String, choices: Vec<Choice>, foot: Vec<String>) -> Self {
        let index = choices.iter().position(|choice| choice.now).unwrap_or(0);
        Chooser { title, question: question.to_string(), explain, choices, index, foot, opens: false }
    }

    fn view(&self, width: usize) -> (Vec<Line<'static>>, Vec<(usize, Spot)>) {
        let mut spots = Vec::new();
        let mut lines = vec![Line::from(Span::styled(self.question.clone(), bold(MAUVE)))];
        lines.extend(wrap(&self.explain, width, 6).into_iter().map(|text| Line::from(Span::styled(text, style(MUTED)))));
        lines.push(Line::default());
        answer_lines(&mut lines, &mut spots, &self.choices, self.index, width);
        lines.push(Line::default());
        if !self.foot.is_empty() {
            for text in &self.foot {
                lines.extend(wrap(text, width, 4).into_iter().map(|text| Line::from(Span::styled(text, style(MUTED)))));
            }
            lines.push(Line::default());
        }
        let mut keys = vec![("↑ ↓ choose", None), ("⏎ set", Some(KeyCode::Enter.into()))];
        keys.extend(self.opens.then(|| ("o open the file", Some(KeyCode::Char('o').into()))));
        keys.push(("esc close", Some(KeyCode::Esc.into())));
        hint_line(&mut lines, &mut spots, &keys);
        (lines, spots)
    }
}

const POPUP: usize = 64;

enum Mode {
    Normal,
    Edit { typed: Typed, buffer: String },
    Filter { buffer: String },
    Team { key: &'static str, change: Change },
    Choose(Box<Chooser>),
    Menu { name: String, index: usize },
    Wizard(Box<Wizard>),
    Delete { name: String, nudged: bool },
}

pub struct Paths {
    pub machine: PathBuf,
    pub repos_dir: PathBuf,
    pub claude_home: PathBuf,
    pub kebbie: PathBuf,
    pub profiles: PathBuf,
    pub codex_home: PathBuf,
    pub path: Vec<PathBuf>,
    pub kebbie_command: String,
    pub sessions: Option<PathBuf>,
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
            path: std::env::var_os("PATH").map(|path| std::env::split_paths(&path).collect()).unwrap_or_default(),
            kebbie_command: crate::peers::kebbie_command().unwrap_or_else(|_| "kebbie".to_string()),
            sessions: None,
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
    hits: Vec<(Rect, Spot)>,
}

#[derive(Default)]
struct Agents {
    profiles: Vec<Profile>,
    ignored: Vec<String>,
    codex_models: Option<Vec<CachedModel>>,
    codex_default: Option<String>,
    claude_default: Option<String>,
    review_model: Option<(String, Layer)>,
    version: String,
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
            version: crate::issue_recheck::version_at(&paths.kebbie).unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string()),
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
        Page { paths, repo, tab: Tab::Machine, section: 0, row: 0, filter: String::new(), mode: Mode::Normal, said: None, entries: Vec::new(), error: None, agents: Agents::default(), hits: Vec::new() }
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

    fn chooser(&self, entry: &Entry) -> Chooser {
        let now = edit_text(&entry.value);
        let (question, choices) = match &entry.kind {
            Kind::Choice(options) => ("Which value?", options.iter().map(|option| {
                let none = *option == "none";
                let label = if none { "none".to_string() } else { format!("{option:?}") };
                choice(&label, "", Then::Write(if none { Change::Remove } else { Change::Set(Set::Text(option.to_string())) }), *option == now || (none && entry.value == "none"))
            }).collect()),
            Kind::Profile => {
                let set = (entry.layer != Layer::Default).then_some(now);
                let mut choices = vec![choice(&format!("none ({})", profiles::TASK), "the built-in task", Then::Write(Change::Remove), set.is_none())];
                choices.extend(self.agents.profiles.iter().filter(|profile| profile.name != profiles::REVIEWER).map(|profile| {
                    let about = if profile.fits.is_empty() { "(for not said)" } else { profile.fits.as_str() };
                    choice(&format!("{:?}", profile.name), about, Then::Write(Change::Set(Set::Text(profile.name.clone()))), set.as_deref() == Some(profile.name.as_str()))
                }));
                keep_current(&mut choices, 0, set.as_deref(), |name| choice(&format!("{name:?}"), "no profile loads by this name", Then::Write(Change::Set(Set::Text(name.to_string()))), true));
                ("Which profile?", choices)
            }
            Kind::Skill => {
                let default = default_skill(entry.key);
                let skill = default.split_once(':').map_or(default, |(_, skill)| skill);
                ("Which skill?", vec![
                    choice(&format!("{default:?}"), "built in", Then::Write(Change::Set(Set::Text(default.to_string()))), false),
                    choice("Your skill, by name", "<skill> or <plugin>:<skill>", Then::Type(Typed::SkillName), false),
                    choice("A Markdown file, by path", "an absolute or ~ path to a .md file", Then::Type(Typed::SkillFile), false),
                    choice(&format!("Copy kebbie's to {}/{skill} and edit it", home_relative(&self.paths.claude_home.join("skills"))), &format!("then the key names \"{skill}\""), Then::Write(Change::Copy(skill)), false),
                ])
            }
            _ => ("", Vec::new()),
        };
        Chooser::new(entry.key.to_string(), question, entry.about.clone(), choices, Vec::new())
    }

    pub fn paste(&mut self, text: &str) -> bool {
        match &mut self.mode {
            Mode::Edit { buffer, .. } | Mode::Filter { buffer } => {
                buffer.push_str(text.trim_end_matches(['\n', '\r']));
                true
            }
            Mode::Wizard(wizard) => wizard.paste(text),
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
                KeyCode::Enter => {
                    if let Some(choice) = chooser.choices.into_iter().nth(chooser.index) {
                        return self.then(choice.then);
                    }
                }
                KeyCode::Char('o') if chooser.opens => match chooser.choices.get(chooser.index).map(|choice| &choice.then) {
                    Some(Then::Role(Role::File(file))) => return Effect::Open(self.paths.profiles.join(file)),
                    _ => {
                        self.say("o opens a role file; this one has none yet".to_string(), MUTED);
                        self.mode = Mode::Choose(chooser);
                    }
                },
                KeyCode::Esc | KeyCode::Char('q') => {}
                _ => self.mode = Mode::Choose(chooser),
            },
            Mode::Menu { name, index } => {
                let keys: Vec<char> = self.menu_choices(&name).iter().map(|choice| choice.key).collect();
                match key.code {
                    KeyCode::Up | KeyCode::Char('k') => self.mode = Mode::Menu { name, index: index.saturating_sub(1) },
                    KeyCode::Down | KeyCode::Char('j') => self.mode = Mode::Menu { index: (index + 1).min(keys.len().saturating_sub(1)), name },
                    KeyCode::Enter => return keys.get(index).map_or(Effect::Handled, |key| self.menu(*key)),
                    KeyCode::Char(key) if keys.contains(&key) => return self.menu(key),
                    KeyCode::Esc | KeyCode::Char('q') => {}
                    _ => self.mode = Mode::Menu { name, index },
                }
            }
            Mode::Wizard(mut wizard) => {
                let step = wizard.step;
                match wizard.key(key) {
                    profile_wizard::Step::Cancel => {}
                    profile_wizard::Step::Write { open } => return self.add(wizard, open),
                    profile_wizard::Step::Stay => {
                        if wizard.step > step {
                            self.offer(&mut wizard);
                        }
                        self.mode = Mode::Wizard(wizard);
                    }
                }
            }
            Mode::Delete { name, .. } => match key.code {
                KeyCode::Char('y') => self.delete(&name),
                KeyCode::Char('n') | KeyCode::Esc | KeyCode::Char('q') => {}
                _ => self.mode = Mode::Delete { name, nudged: true },
            },
            Mode::Team { key: name, change } => match key.code {
                KeyCode::Char('t') => _ = self.apply(name, change, false),
                KeyCode::Char('o') => _ = self.apply(name, change, true),
                KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('q') => {}
                _ => self.mode = Mode::Team { key: name, change },
            },
        }
        Effect::Handled
    }

    fn then(&mut self, then: Then) -> Effect {
        match (then, self.current()) {
            (Then::Field(field, value), _) => self.set_field(field, value.as_deref()),
            (Then::TypeModel, _) => self.mode = Mode::Edit { typed: Typed::Model, buffer: self.profile().and_then(|profile| self.agents.own_model(&profile)).unwrap_or_default() },
            (Then::Role(to), _) => return self.set_role(to),
            (Then::Write(change), Some(entry)) => self.request(entry.key, change),
            (Then::Type(typed), Some(entry)) => {
                let now = edit_text(&entry.value);
                let keep = match typed { Typed::SkillFile => config::skill_path(&now).is_some(), _ => now != default_skill(entry.key) && config::skill_path(&now).is_none() };
                self.mode = Mode::Edit { typed, buffer: if keep { now } else { String::new() } };
            }
            _ => {}
        }
        Effect::Handled
    }

    pub fn mouse(&mut self, event: MouseEvent) -> Effect {
        if event.kind != MouseEventKind::Down(MouseButton::Left) {
            return Effect::Handled;
        }
        let hit = self.hits.iter().find(|(area, _)| area.contains(Position::new(event.column, event.row))).map(|(_, spot)| *spot);
        if let (Some(Spot::Choice(at)), Mode::Menu { index, .. }) = (hit, &mut self.mode) {
            *index = at;
            return self.handle(KeyCode::Enter.into());
        }
        if let (Some(Spot::Choice(at)), Mode::Wizard(wizard)) = (hit, &mut self.mode) {
            return if wizard.choose(at) { self.handle(KeyCode::Enter.into()) } else { Effect::Handled };
        }
        let closable = matches!(self.mode, Mode::Choose(_) | Mode::Menu { .. });
        let selected = match &mut self.mode {
            Mode::Choose(chooser) => Some(&mut chooser.index),
            _ => None,
        };
        match (hit, selected) {
            (Some(Spot::Choice(at)), Some(index)) if *index != at => *index = at,
            (Some(Spot::Choice(_)), _) => return self.handle(KeyCode::Enter.into()),
            (Some(Spot::Key { key, .. }), _) => return self.handle(key),
            (None, _) if closable => self.mode = Mode::Normal,
            _ => {}
        }
        Effect::Handled
    }

    fn handle_normal(&mut self, key: KeyEvent) -> Effect {
        if self.on_profiles() {
            match key.code {
                KeyCode::Char('m') => return self.choose(Field::Model),
                KeyCode::Char('e') => return self.choose(Field::Effort),
                KeyCode::Char('r') => return self.role(),
                KeyCode::Char('n') => {
                    let name = self.profile().map(|profile| profile.name).unwrap_or_default();
                    self.wizard(&name);
                    return Effect::Handled;
                }
                KeyCode::Char('x') => {
                    self.ask_delete();
                    return Effect::Handled;
                }
                KeyCode::Char('d') => {
                    let reviewed = self.profile().and_then(|profile| self.agents.review_model(&profile).map(str::to_string)).is_some();
                    let removed = reviewed.then(|| {
                        let done = self.apply("review.model", Change::Remove, false).is_ok();
                        (self.said.take().map(|(text, _)| text).unwrap_or_default(), done)
                    });
                    _ = self.write_profile(&[("model", None), ("effort", None)]);
                    if let Some((removed, done)) = removed && let Some((text, color)) = self.said.take() {
                        self.say(format!("{removed} · {text}"), if done { color } else { RED });
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
            Kind::Choice(_) | Kind::Skill | Kind::Profile => self.mode = Mode::Choose(Box::new(self.chooser(&entry))),
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
        _ = self.apply(key, change, false);
    }

    fn apply(&mut self, key: &'static str, change: Change, replace_link: bool) -> Result<()> {
        let file = self.file();
        let machine = self.tab == Tab::Machine || self.repo.is_none();
        let name = file.file_name().map(|name| name.to_string_lossy().to_string()).unwrap_or_default();
        let shown = if machine { name.clone() } else { format!("repos/{name}") };
        let (change, copied) = match change {
            Change::Copy(skill) => match copy_skill(&self.paths.kebbie, &self.paths.claude_home, skill) {
                Ok((to, true)) => (Change::Set(Set::Text(skill.to_string())), format!(" · copied kebbie's skill to {}: edit {}", home_relative(&to), home_relative(&to.join("SKILL.md")))),
                Ok((to, false)) => (Change::Set(Set::Text(skill.to_string())), format!(" · {} was already there, so nothing was copied: edit its SKILL.md", home_relative(&to))),
                Err(error) => return self.fail(format!("✗ copying kebbie's {skill}: {error}")),
            },
            other => (other, String::new()),
        };
        let written = match &change {
            Change::Set(set) => config_edit::write(&file, machine, replace_link, |text| config_edit::set(text, key, set)).map(|()| format!("✓ wrote {key} = {} to {shown}", set.shown())),
            _ => config_edit::write(&file, machine, replace_link, |text| config_edit::remove(text, key)).map(|()| format!("✓ removed {key} from {shown}; the layer below applies")),
        };
        let done = match written {
            Ok(said) => {
                let check = if machine { crate::doctor::config_file(&file).0 } else { crate::doctor::repo_file(&file).0 };
                let level = match check.level { crate::doctor::Level::Ok => "ok", crate::doctor::Level::Warn => "warn", crate::doctor::Level::Fail => "fail" };
                let fact = check.fact.replace(&file.display().to_string(), &shown);
                let color = if check.level == crate::doctor::Level::Ok { GREEN } else { PEACH };
                let linked = if replace_link { " (now a file of this machine's own)" } else { "" };
                self.say(format!("{said}{linked}{copied} · doctor: {level}, {fact}"), color);
                Ok(())
            }
            Err(error) => self.fail(format!("✗ {}", error.lines().next().unwrap_or_default())),
        };
        self.load();
        done
    }

    fn fail(&mut self, text: String) -> Result<()> {
        self.say(text.clone(), RED);
        Err(text)
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
        _ = self.write_profile(&[(field.key(), value)]);
    }

    fn foot(&self, profile: &Profile, field: Field) -> Vec<String> {
        let key = field.key();
        if field == Field::Model && profile.name == profiles::REVIEWER {
            let file = home_relative(&self.file());
            return vec![format!("This writes review.model in {file}, ahead of the reviewer profile's model."), "A review already running keeps its model; the next review takes it.".to_string()];
        }
        let running = format!("A task already running keeps its {key}; the next start takes it.");
        if self.built_in_without_file(profile) {
            return vec![format!("{name} is kebbie's: a {key} you set goes in profiles/{name}.toml, which an update never writes, and kebbie's role and rules stay as they ship. d on the row removes that file again.", name = profile.name), running];
        }
        vec![running]
    }

    fn model_choices(&self, agent: Agent, current: Option<&str>, reviewer: Option<&Profile>) -> (String, Vec<Choice>) {
        let agents = &self.agents;
        match agent {
            Agent::Codex => {
                let cache = home_relative(&self.paths.codex_home.join("models_cache.json"));
                let explain = if agents.codex_models.is_some() { "Codex's own list for your account.".to_string() } else { format!("No list from Codex at {cache}: Codex writes it when it starts.") };
                let mut choices: Vec<Choice> = agents.codex_models.iter().flatten().filter(|model| model.listed).map(|model| choice(&model.name, &model.slug, Then::Field(Field::Model, Some(model.slug.clone())), current == Some(model.slug.as_str()))).collect();
                let default = match agents.codex_default.as_deref() {
                    Some(model) if agents.offered(model) == Some(false) => format!("{model}, not offered to you ⚠"),
                    Some(model) => model.to_string(),
                    None => "none in Codex's config.toml: Codex picks".to_string(),
                };
                choices.push(choice("Codex's default", &default, Then::Field(Field::Model, None), current.is_none()));
                choices.push(choice("Type a model…", "any id Codex accepts", Then::TypeModel, false));
                keep_current(&mut choices, 2, current, |model| choice(model, if agents.offered(model) == Some(false) { "not offered to you ⚠" } else { "not in Codex's list" }, Then::Field(Field::Model, Some(model.to_string())), true));
                (explain, choices)
            }
            Agent::Claude => {
                let settings = home_relative(&self.paths.claude_home.join("settings.json"));
                let default = match agents.claude_default.as_deref() {
                    Some(model) => format!("{model}, from {settings}"),
                    None => format!("none in {settings}: Claude Code picks"),
                };
                let (label, default) = match reviewer.and_then(|profile| profile.model.as_ref()) {
                    Some(model) => ("The profile's".to_string(), format!("{model}, from reviewer.toml")),
                    None => ("Claude's default".to_string(), default),
                };
                let mut choices = vec![choice(&label, &default, Then::Field(Field::Model, None), current.is_none())];
                choices.extend(CLAUDE_MODELS.iter().map(|(name, about)| choice(name, about, Then::Field(Field::Model, Some(name.to_string())), current == Some(*name))));
                choices.push(choice("Type a model…", "a full id, as claude-opus-5-5", Then::TypeModel, false));
                keep_current(&mut choices, 1, current, |model| choice(model, "a model id of its own", Then::Field(Field::Model, Some(model.to_string())), true));
                let explain = if reviewer.is_some() { "Claude Code's model names; this sets review.model, which kebbie review runs.".to_string() } else { "Claude Code's model names.".to_string() };
                (explain, choices)
            }
        }
    }

    fn effort_choices(&self, agent: Agent, model: Option<&str>, current: Option<&str>) -> (String, Vec<Choice>) {
        let agents = &self.agents;
        let cached = model.and_then(|model| agents.cached(model)).filter(|_| agent == Agent::Codex);
        let shown = model.map_or_else(|| format!("{}'s default", agent.name()), str::to_string);
        let (levels, default, explain): (Vec<String>, Option<String>, String) = match (agent, cached) {
            (Agent::Codex, Some(cached)) => (cached.efforts.iter().filter(|level| profiles::CODEX_EFFORTS.contains(&level.as_str())).cloned().collect(), cached.default_effort.clone(), format!("The levels {} takes, from Codex's list.", cached.name)),
            (Agent::Codex, None) => (profiles::CODEX_EFFORTS.map(str::to_string).to_vec(), None, format!("{shown} is not in Codex's list, so these are every level Codex takes.")),
            (Agent::Claude, _) => (profiles::EFFORTS.map(str::to_string).to_vec(), None, format!("Claude Code's effort levels, for {shown}.")),
        };
        let mut choices: Vec<Choice> = levels.iter().map(|level| {
            let about = if default.as_deref() == Some(level.as_str()) { "Codex's default for this model" } else if level == "ultra" { "only some Codex models" } else { "" };
            choice(level, about, Then::Field(Field::Effort, Some(level.clone())), current == Some(level.as_str()))
        }).collect();
        let leave = match (agent, &default) {
            (Agent::Codex, Some(level)) => format!("leave it to Codex: {level}"),
            _ => format!("leave it to {}", agent.name()),
        };
        choices.push(choice("Default", &leave, Then::Field(Field::Effort, None), current.is_none()));
        keep_current(&mut choices, 1, current, |level| choice(level, "not a level this model lists", Then::Field(Field::Effort, Some(level.to_string())), true));
        (explain, choices)
    }

    fn choose(&mut self, field: Field) -> Effect {
        let Some(profile) = self.profile() else { return Effect::Handled };
        let reviewer = profile.name == profiles::REVIEWER;
        let (question, (explain, choices)) = match field {
            Field::Model => {
                let current = if reviewer { self.agents.review_model(&profile).map(str::to_string) } else { profile.model.clone() };
                ("Which model?", self.model_choices(profile.agent, current.as_deref(), reviewer.then_some(&profile)))
            }
            Field::Effort => ("How much effort?", self.effort_choices(profile.agent, self.agents.model(&profile).as_deref(), profile.effort.as_deref())),
        };
        let foot = self.foot(&profile, field);
        self.said = None;
        self.mode = Mode::Choose(Box::new(Chooser::new(format!("{} · {}", field.key(), profile.name), question, explain, choices, foot)));
        Effect::Handled
    }

    fn open_profile(&mut self) -> Effect {
        let Some(profile) = self.profile() else { return Effect::Handled };
        match profile.file {
            Some(file) => Effect::Open(file),
            None => {
                self.say(format!("{} is kebbie's and has no file of yours until you set something: m, e or r writes profiles/{}.toml", profile.name, profile.name), MUTED);
                Effect::Handled
            }
        }
    }

    pub fn opened(&mut self, file: &Path, result: Result<bool>) {
        self.load();
        let shown = self.shown_file(file);
        match result {
            Ok(_) if file.extension().is_some_and(|extension| extension == "md") => {
                let users: Vec<String> = self.agents.profiles.iter().filter(|profile| profile.role.as_deref() == Some(file)).map(|profile| profile.name.clone()).collect();
                let read = if users.is_empty() { "no profile reads it".to_string() } else { format!("read after kebbie's role by {}", users.join(", ")) };
                self.say(format!("{shown}: {read}"), GREEN);
            }
            Ok(_) => match self.agents.ignored.iter().find(|error| error.contains(&file.display().to_string())) {
                Some(error) => self.say(format!("✗ {}", error.replace(&file.display().to_string(), &shown)), RED),
                None => self.say(format!("{shown} loads"), GREEN),
            },
            Err(error) => self.say(format!("✗ the editor: {error}"), RED),
        }
    }

    pub fn shown(&mut self, result: Result<bool>) {
        if let Err(error) = result {
            self.say(format!("✗ the pager: {error}"), RED);
        }
    }

    fn role(&mut self) -> Effect {
        let Some(profile) = self.profile() else { return Effect::Handled };
        self.said = None;
        if profiles::built_in(&profile.name).is_some() {
            self.mode = Mode::Menu { name: profile.name, index: 0 };
            return Effect::Handled;
        }
        let dir = &self.paths.profiles;
        let current = profiles::role_name(dir, &profile);
        let new = profiles::free_role(dir, &profile.name);
        let (mine, others): (Vec<_>, Vec<_>) = profiles::role_choices(dir, &self.agents.profiles, &profile.name).into_iter().partition(|(file, _)| Some(file) == current.as_ref());
        let mut choices: Vec<Choice> = mine.into_iter().chain(others).map(|(file, about)| choice(&file, &about, Then::Role(Role::File(file.clone())), Some(&file) == current.as_ref())).collect();
        choices.push(choice("New role", &format!("{new}, an empty file"), Then::Role(Role::New(new.clone())), false));
        if let Some(current) = &current {
            choices.push(choice(&format!("Copy of {current}"), "to change it for this profile alone", Then::Role(Role::Copy { from: current.clone(), to: new }), false));
        }
        choices.push(choice("None", "kebbie's role alone", Then::Role(Role::None), current.is_none()));
        let explain = format!("Your lines, read after {}.", profiles::kebbie_role(&profile.name).0);
        let mut chooser = Chooser::new(format!("role · {}", profile.name), "Which role?", explain, choices, vec!["A shared file changes for every profile that uses it.".to_string()]);
        chooser.opens = true;
        self.mode = Mode::Choose(Box::new(chooser));
        Effect::Handled
    }

    // A role file made here takes a name no file has, so it never replaces one the user has.
    fn set_role(&mut self, to: Role) -> Effect {
        let dir = self.paths.profiles.clone();
        let made = match &to {
            Role::New(file) => Some((file.clone(), Ok(String::new()))),
            Role::Copy { from, to } => Some((to.clone(), std::fs::read_to_string(dir.join(from)).map_err(|error| format!("✗ {from}: {error}")))),
            _ => None,
        };
        if let Some((file, text)) = &made {
            let written = text.clone().and_then(|text| if dir.join(file).exists() { Err(format!("✗ {file} is in the folder already")) } else { crate::state::write(&dir.join(file), text).map_err(|error| format!("✗ {file}: {error}")) });
            if let Err(error) = written {
                _ = self.fail(error);
                return Effect::Handled;
            }
        }
        let written = self.write_profile(&[("role", to.file())]);
        match made {
            Some((file, _)) if written.is_err() => {
                let _ = std::fs::remove_file(dir.join(file));
                Effect::Handled
            }
            Some((file, _)) => Effect::Open(dir.join(file)),
            None => Effect::Handled,
        }
    }

    fn menu_choices(&self, name: &str) -> Vec<widgets::Choice> {
        let row = |key: char, label: String, about: String| widgets::Choice { key, act: None, label, about, link: None };
        let file = profiles::kebbie_role(name).1;
        let every = if name == profiles::REVIEWER { "every review" } else { "every task" };
        let own = self.agents.profiles.iter().find(|profile| profile.name == name).and_then(|profile| profiles::role_name(&self.paths.profiles, profile)).unwrap_or_else(|| format!("{name}.md"));
        let others: Vec<String> = profiles::role_readers(&self.paths.profiles, &own).into_iter().filter(|reader| reader != name).collect();
        let shared = if others.is_empty() { String::new() } else { format!(", shared with {}", others.join(", ")) };
        let mut choices = vec![
            row('s', "Show kebbie's role".to_string(), format!("{file}, read-only")),
            row('a', "Add your lines".to_string(), format!("profiles/{own}{shared}, read after kebbie's role by {every}")),
        ];
        if name != profiles::REVIEWER {
            choices.push(row('n', format!("New profile from {name}"), format!("the new-profile steps, starting from {name}")));
        }
        choices
    }

    fn wizard(&mut self, from: &str) {
        let starts: Vec<Profile> = self.agents.profiles.iter().filter(|profile| profile.name != profiles::REVIEWER).cloned().collect();
        let start = starts.iter().position(|profile| profile.name == from).or_else(|| starts.iter().position(|profile| profile.name == profiles::TASK)).unwrap_or(0);
        let dir = &self.paths.profiles;
        let mut taken: Vec<String> = self.agents.profiles.iter().map(|profile| profile.name.clone()).collect();
        taken.extend(std::fs::read_dir(dir).into_iter().flatten().flatten().filter_map(|entry| entry.file_name().to_str()?.strip_suffix(".toml").map(str::to_string)));
        let codex = crate::agent::codex::availability(&self.paths.path, Some(&self.paths.codex_home), &self.paths.kebbie_command).short_line();
        let files = profiles::role_choices(dir, &self.agents.profiles, "");
        self.said = None;
        self.mode = Mode::Wizard(Box::new(Wizard::new(starts, start, files, dir.clone(), home_relative(dir), taken, codex)));
    }

    fn answers(choices: Vec<Choice>) -> (Vec<Answer<Pick>>, usize) {
        let selected = choices.iter().position(|choice| choice.now).unwrap_or(0);
        let answers = choices.into_iter().map(|choice| Answer::new(&choice.label, &choice.about, match choice.then { Then::TypeModel => Pick::Type, Then::Field(_, value) => Pick::Value(value), _ => Pick::Value(None) })).collect();
        (answers, selected)
    }

    fn offer(&self, wizard: &mut Wizard) {
        let start = wizard.from().filter(|profile| profile.agent == wizard.agent()).cloned();
        match wizard.step {
            3 => {
                let (explain, choices) = self.model_choices(wizard.agent(), start.as_ref().and_then(|profile| profile.model.as_deref()), None);
                let (answers, selected) = Self::answers(choices);
                wizard.offer_models(explain, answers, selected);
            }
            4 => {
                let model = wizard.model().or_else(|| self.agents.default_model(wizard.agent()).map(str::to_string));
                let (explain, choices) = self.effort_choices(wizard.agent(), model.as_deref(), start.as_ref().and_then(|profile| profile.effort.as_deref()));
                let (answers, selected) = Self::answers(choices);
                wizard.offer_efforts(explain, answers, selected);
            }
            _ => {}
        }
    }

    // The role file goes first, as the profile parses only with its role there; a role file this call wrote goes again when the profile is not written.
    fn add(&mut self, mut wizard: Box<Wizard>, open: bool) -> Effect {
        let name = wizard.name.trim().to_string();
        let dir = self.paths.profiles.clone();
        let file = dir.join(format!("{name}.toml"));
        let Some(made) = wizard.made() else { return Effect::Handled };
        let role = wizard.chosen_role();
        let made_role = match &role {
            Role::New(to) => Ok(Some((to.clone(), String::new()))),
            Role::Copy { from, to } => std::fs::read_to_string(dir.join(from)).map(|text| Some((to.clone(), text))).map_err(|error| format!("{from}: {error}")),
            _ => Ok(None),
        };
        let mut wrote = None;
        let written = made_role.and_then(|made_role| {
            if file.exists() {
                return Err(format!("{name}.toml is in the folder already"));
            }
            if let Some((to, text)) = made_role {
                if dir.join(&to).exists() {
                    return Err(format!("{to} is in the folder already"));
                }
                crate::state::write(&dir.join(&to), text)?;
                wrote = Some(to);
            }
            profiles::parse(&made.text, &name, &file, None)?;
            crate::state::write(&file, &made.text)
        });
        if let Err(error) = written {
            if let Some(to) = wrote {
                let _ = std::fs::remove_file(dir.join(to));
            }
            wizard.error = Some(error.replace(&dir.display().to_string(), "profiles"));
            self.mode = Mode::Wizard(wizard);
            return Effect::Handled;
        }
        self.load();
        if let Some(row) = self.agents.profiles.iter().position(|profile| profile.name == name) {
            self.row = row;
        }
        let role_said = role.file().map_or_else(String::new, |file| format!(", its role {file}"));
        self.say(format!("✓ added profiles/{name}.toml{role_said} · kebbie start --profile {name}, or task.profile"), GREEN);
        match (open, role.file()) {
            (false, _) => Effect::Handled,
            (true, Some(role)) => Effect::Open(dir.join(role)),
            (true, None) => Effect::Open(file),
        }
    }

    fn sessions(&self) -> Option<PathBuf> {
        self.paths.sessions.clone().or_else(|| crate::lead::state_location().ok().map(|state| state.join("sessions")))
    }

    fn kept(&self, profile: &Profile) -> Option<String> {
        if profiles::built_in(&profile.name).is_some() {
            return Some(format!("✗ {} is kebbie's and stays; d removes your settings on it", profile.name));
        }
        let open = self.sessions().map(|sessions| crate::tasks::on_profile(&sessions, &profile.name)).unwrap_or_default();
        if !open.is_empty() {
            return Some(format!("✗ {} is the profile of {}, still open; it can go once {} closes", profile.name, open.join(", "), if open.len() == 1 { "that task" } else { "they" }));
        }
        let machine = config::check_machine(&self.paths.machine).ok().map(|(machine, _)| machine);
        if let Some((file, _)) = crate::doctor::named_profiles(machine.as_ref(), &self.paths.machine, &self.paths.repos_dir).into_iter().find(|(_, named)| *named == profile.name) {
            return Some(format!("✗ task.profile names {} in {file}, so every new task there starts with it; pick another first", profile.name));
        }
        let routines: Vec<String> = crate::routines::load_from(&self.paths.machine, &self.paths.repos_dir).0.into_iter().filter(|routine| routine.profile.as_deref() == Some(profile.name.as_str())).map(|routine| format!("\"{}\"", routine.title)).collect();
        if !routines.is_empty() {
            return Some(format!("✗ {} is the profile the routine {} starts its tasks with; give {} another first", profile.name, routines.join(", "), if routines.len() == 1 { "it" } else { "them" }));
        }
        None
    }

    fn deletion(&self, profile: &Profile) -> (String, String, bool) {
        let file = profile.file.as_deref().map(|file| self.shown_file(file)).unwrap_or_default();
        let Some(role) = profiles::role_name(&self.paths.profiles, profile) else { return (format!("{file} goes."), format!("✓ removed {file}"), false) };
        let others: Vec<String> = profiles::role_readers(&self.paths.profiles, &role).into_iter().filter(|reader| *reader != profile.name).collect();
        match others.as_slice() {
            [] => (format!("{file} goes, and its role {role}, which no other profile reads."), format!("✓ removed {file} and its role {role}"), true),
            others => (format!("{file} goes; its role {role} stays, as {} reads it too.", others.join(", ")), format!("✓ removed {file}; its role {role} stays for {}", others.join(", ")), false),
        }
    }

    fn ask_delete(&mut self) {
        let Some(profile) = self.profile() else { return };
        match self.kept(&profile) {
            Some(why) => self.say(why, RED),
            None => self.mode = Mode::Delete { name: profile.name, nudged: false },
        }
    }

    fn delete(&mut self, name: &str) {
        let Some(profile) = self.agents.profiles.iter().find(|profile| profile.name == name).cloned() else { return };
        if let Some(why) = self.kept(&profile) {
            return self.say(why, RED);
        }
        let (_, said, role_goes) = self.deletion(&profile);
        let removed = profile.file.as_deref().map(std::fs::remove_file).transpose().and_then(|_| match &profile.role {
            Some(role) if role_goes && role.starts_with(&self.paths.profiles) => std::fs::remove_file(role),
            _ => Ok(()),
        });
        self.load();
        match removed {
            Ok(()) => self.say(said, GREEN),
            Err(error) => self.say(format!("✗ {name}: {error}"), RED),
        }
    }

    fn menu(&mut self, key: char) -> Effect {
        let Some(profile) = self.profile() else { return Effect::Handled };
        match key {
            's' => {
                let file = self.paths.kebbie.join(profiles::kebbie_role(&profile.name).1);
                if file.is_file() {
                    return Effect::Show(file);
                }
                self.say(format!("✗ {} is not in kebbie's install", home_relative(&file)), RED);
            }
            'n' => self.wizard(&profile.name),
            'a' => {
                if let Some(role) = profile.role {
                    return Effect::Open(role);
                }
                let file = format!("{}.md", profile.name);
                let path = self.paths.profiles.join(&file);
                let made = !path.exists();
                if made && let Err(error) = crate::state::write(&path, String::new()) {
                    _ = self.fail(format!("✗ {file}: {error}"));
                    return Effect::Handled;
                }
                if self.write_profile(&[("role", Some(&file))]).is_err() {
                    if made {
                        let _ = std::fs::remove_file(&path);
                    }
                    return Effect::Handled;
                }
                return Effect::Open(path);
            }
            _ => {}
        }
        Effect::Handled
    }

    fn model_said(&self, profile: &Profile) -> String {
        match (self.agents.review_model.as_ref().filter(|_| profile.name == profiles::REVIEWER), &profile.model, self.agents.default_model(profile.agent)) {
            (Some((model, layer)), _, _) => format!("{model} from review.model ({})", self.layer_word(*layer).0),
            (None, Some(model), _) => model.clone(),
            (None, None, Some(model)) => format!("{model} by {}'s default", profile.agent.name()),
            (None, None, None) => format!("{}'s default model", profile.agent.name()),
        }
    }

    fn runs(&self, profile: &Profile) -> String {
        let model = self.model_said(profile);
        let effort = profile.effort.as_deref().map_or_else(String::new, |effort| format!(", effort {effort}"));
        format!("{} runs {} on {model}{effort}", profile.name, profile.agent.key())
    }

    fn summary(&self, profile: &Profile) -> String {
        let model = self.model_said(profile);
        let mut said = vec![format!("{}: {}, {model}", profile.name, profile.agent.key())];
        said.extend(profile.effort.clone());
        let mut text = said.join(", ");
        if let Some(role) = profiles::role_name(&self.paths.profiles, profile) {
            text.push_str(&format!(" · role {role}"));
        }
        if profiles::built_in(&profile.name).is_some() && let Some(file) = &profile.file {
            text.push_str(&format!(" · your settings in {}", self.shown_file(file)));
        }
        text
    }

    fn write_profile(&mut self, changes: &[(&str, Option<&str>)]) -> Result<()> {
        let Some(profile) = self.profile() else { return Ok(()) };
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
            Err(error) => return self.fail(format!("✗ {}", error.lines().next().unwrap_or_default().replace(&self.paths.profiles.display().to_string(), "profiles"))),
        }
        Ok(())
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
            return (
                line(&["↑ ↓ profile", "← → section", "m model", "e effort", "r role", "n new", "⏎ open", "d default", "x delete", "tab machine / repo", "/ search", "esc board"]),
                line(&["↑ ↓ profile", "m model", "e effort", "r role", "n new", "tab repo", "esc board"]),
            );
        }
        let change = match self.current().map(|entry| entry.kind) {
            Some(Kind::Bool) => "space toggle",
            Some(Kind::Choice(_) | Kind::Skill | Kind::Profile) => "⏎ pick",
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

    fn adapter(&self, profile: &Profile, chosen: bool) -> Vec<Span<'static>> {
        let agents = &self.agents;
        let dot = || Span::styled(" · ", style(MUTED));
        let mut spans = vec![Span::styled(profile.agent.key().to_string(), style(if profile.agent == Agent::Codex { BLUE } else { TEAL }))];
        let warned = agents.warning(profile).is_some();
        match (&self.mode, chosen) {
            (Mode::Edit { typed: Typed::Model, buffer }, true) => spans.extend([dot(), Span::styled(format!("{buffer}▏"), bold(TEXT))]),
            _ => {
                if let Some(model) = agents.model(profile) {
                    spans.extend([dot(), Span::styled(model, style(if warned { RED } else if agents.own_model(profile).is_some() { TEXT } else { MUTED }))]);
                }
                if warned {
                    spans.push(Span::styled(" ⚠", style(RED)));
                }
            }
        }
        if let Some(effort) = &profile.effort {
            spans.extend([dot(), Span::styled(effort.clone(), style(TEXT))]);
        }
        spans
    }

    fn role_spans(&self, profile: &Profile) -> Vec<Span<'static>> {
        let own = profiles::role_name(&self.paths.profiles, profile).map(|role| Span::styled(format!("+ {role}"), style(BLUE)));
        match (profiles::built_in(&profile.name).is_some(), own) {
            (true, Some(own)) => vec![Span::styled(format!("{} ", profiles::kebbie_role(&profile.name).0), style(MUTED)), own],
            (_, Some(own)) => vec![own],
            (_, None) => vec![Span::styled(profiles::kebbie_role(&profile.name).0.to_string(), style(MUTED))],
        }
    }

    fn profile_lines(&self, width: usize) -> (Vec<Line<'static>>, usize) {
        let agents = &self.agents;
        let name_width = agents.profiles.iter().map(|profile| text_width(&profile.name)).max().unwrap_or(0).min(width / 4) + 2;
        let adapters: Vec<Vec<Span<'static>>> = agents.profiles.iter().enumerate().map(|(at, profile)| self.adapter(profile, at == self.row)).collect();
        let adapter_width = adapters.iter().map(|spans| spans.iter().map(Span::width).sum::<usize>()).max().unwrap_or(0) + 3;
        let (mut lines, mut selected) = (Vec::new(), 0);
        let kebbies = agents.profiles.iter().filter(|profile| profiles::built_in(&profile.name).is_some()).count();
        let heading = |lines: &mut Vec<Line<'static>>, title: &str, about: String| {
            let mut line = group_line(title, if title == "YOURS" { agents.profiles.len() - kebbies } else { kebbies }, TEAL);
            line.spans.insert(0, Span::raw(" "));
            lines.push(line);
            lines.push(Line::from(Span::styled(format!("  {about}"), style(MUTED))));
        };
        heading(&mut lines, "FROM KEBBIE", "follow kebbie's updates; your settings on them are kept".to_string());
        let mut yours = false;
        for (at, profile) in agents.profiles.iter().enumerate() {
            if !yours && profiles::built_in(&profile.name).is_none() {
                yours = true;
                lines.push(Line::default());
                heading(&mut lines, "YOURS", format!("{} · an update never writes here", home_relative(&self.paths.profiles)));
            }
            let chosen = at == self.row;
            let used: usize = adapters[at].iter().map(Span::width).sum();
            let mut spans = vec![
                if chosen { Span::styled("▌", style(MAUVE)) } else { Span::raw(" ") },
                Span::styled(format!(" {:<name_width$}", cut(&profile.name, name_width - 2)), if chosen { bold(TEXT) } else { style(TEXT) }),
            ];
            spans.extend(adapters[at].clone());
            spans.push(Span::raw(" ".repeat(adapter_width.saturating_sub(used))));
            spans.extend(self.role_spans(profile));
            let line = Line::from(spans);
            if !chosen {
                lines.push(line);
                continue;
            }
            selected = lines.len();
            lines.push(line.style(Style::new().bg(SURFACE)));
            let kebbies = profiles::kebbie_role(&profile.name).0;
            lines.push(Line::from(vec![Span::styled("    role:  ", style(SUBTEXT)), Span::styled(format!("{kebbies}, from kebbie {}, updates with kebbie", agents.version), style(SUBTEXT))]));
            if let Some(role) = profiles::role_name(&self.paths.profiles, profile) {
                lines.push(Line::from(vec![Span::raw("         "), Span::styled(format!("+ {role}"), style(BLUE)), Span::styled(", yours: read after kebbie's role", style(SUBTEXT))]));
            }
            let mut keys = vec![Span::raw("    ")];
            let last = if profiles::built_in(&profile.name).is_some() { ("d", "default") } else { ("x", "delete") };
            for (number, (key, word)) in [("m", "model"), ("e", "effort"), ("r", "role"), ("n", "new profile"), ("⏎", "open file"), last].into_iter().enumerate() {
                keys.extend([Span::raw(if number == 0 { "" } else { "  " }), Span::styled(key, style(MAUVE)), Span::styled(format!(" {word}"), style(SUBTEXT))]);
            }
            lines.push(Line::from(keys));
            lines.push(Line::default());
        }
        if !yours {
            lines.push(Line::default());
            heading(&mut lines, "YOURS", format!("{} · an update never writes here", home_relative(&self.paths.profiles)));
            lines.push(Line::from(Span::styled("  none yet: n makes one", style(MUTED))));
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
            Mode::Normal if self.said.is_none() && self.on_profiles() => self.profile().map(|profile| match self.agents.warning(&profile) {
                Some(warning) => (warning, YELLOW),
                None => (self.summary(&profile), SUBTEXT),
            }),
            _ => self.said.clone(),
        };
        if let Some((text, color)) = under {
            let lines: Vec<Line> = wrap(&text, said.width as usize, 2).into_iter().map(|text| Line::from(Span::styled(text, style(color)))).collect();
            frame.render_widget(Paragraph::new(lines), said);
        }
        self.hits.clear();
        match &self.mode {
            Mode::Choose(chooser) => {
                let width = POPUP.min(frame.area().width.saturating_sub(8) as usize);
                let (lines, spots) = chooser.view(width);
                self.hits = modal_spots(frame, chooser.title.clone(), lines, spots, width).1;
            }
            Mode::Wizard(wizard) => {
                let width = POPUP.min(frame.area().width.saturating_sub(8) as usize);
                let (lines, spots) = wizard.view(width);
                self.hits = modal_spots(frame, "new profile".to_string(), lines, spots, width).1;
            }
            Mode::Delete { name, nudged } => {
                let about = self.agents.profiles.iter().find(|profile| &profile.name == name).map(|profile| self.deletion(profile).0).unwrap_or_default();
                modal(frame, "delete".to_string(), widgets::confirm_lines(name, Some(&about), "Delete this profile?", *nudged), 72);
            }
            Mode::Menu { name, index } => {
                let intro = wrap(&format!("{name} is kebbie's: its role comes with kebbie and an update replaces it, so it is not edited here."), 56, 3);
                let mut lines: Vec<Line> = intro.into_iter().map(|text| Line::from(Span::styled(text, style(TEXT)))).collect();
                lines.push(Line::default());
                let choices = self.menu_choices(name);
                let spots = (0..choices.len()).flat_map(|at| [(lines.len() + 2 * at, Spot::Choice(at)), (lines.len() + 2 * at + 1, Spot::Choice(at))]).collect();
                lines.extend(options_lines(choices, *index, 56));
                self.hits = modal_spots(frame, format!("role · {name}"), lines, spots, 58).1;
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

// kebbie's own files are shown, never edited: an update would replace the change.
pub fn show_in_pager(file: &Path) -> Result<bool> {
    std::process::Command::new("sh").args(["-c", "${PAGER:-less} \"$1\"", "sh"]).arg(file).status().map(|status| status.success()).map_err(|error| error.to_string())
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
                        Effect::Show(file) => {
                            let status = crate::board::terminal::outside(&mut terminal, || show_in_pager(&file));
                            page.shown(status);
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
        let paths = Paths { machine: dir.join("config.toml"), repos_dir: dir.join("repos"), claude_home: dir.join("claude"), kebbie, profiles: dir.join("profiles"), codex_home: dir.join("codex"), path: vec![dir.join("bin")], kebbie_command: "kebbie".to_string(), sessions: Some(dir.join("state/sessions")) };
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
        assert!(shown.contains("╭ task.also_noticed ") && shown.contains("│ Which value?") && shown.contains("│ What a task's Claude does with what it notices"), "the key is the title, its text the explanation: {shown}");
        assert!(shown.contains("│ ▌\"backlog\"           now ") && shown.contains("│  \"ask\"  ") && shown.contains("│ ↑ ↓ choose · ⏎ set · esc close"), "{shown}");
        press(&mut fixture.page, &[KeyCode::Down, KeyCode::Down, KeyCode::Enter]);
        assert_eq!(file(&fixture, "config.toml"), "[task]\nalso_noticed = \"ask\"\n");
        go_to(&mut fixture.page, "task.permission_mode");
        press(&mut fixture.page, &[KeyCode::Enter, KeyCode::Down, KeyCode::Down, KeyCode::Enter]);
        assert!(file(&fixture, "config.toml").contains("permission_mode = \"acceptEdits\""));
        press(&mut fixture.page, &[KeyCode::Enter]);
        assert!(matches!(&fixture.page.mode, Mode::Choose(chooser) if chooser.index == 2), "the pick opens on the value in effect");
        press(&mut fixture.page, &[KeyCode::Up, KeyCode::Up, KeyCode::Enter]);
        assert!(!file(&fixture, "config.toml").contains("permission_mode"), "none leaves it to Claude: the key goes");
    }

    fn click(page: &mut Page, shown: &str, text: &str) -> Effect {
        let (row, line) = shown.lines().enumerate().find(|(_, line)| line.contains(text)).unwrap_or_else(|| panic!("{text}: {shown}"));
        let column = line.chars().collect::<Vec<_>>().windows(text.chars().count()).position(|window| window.iter().collect::<String>() == text).unwrap();
        page.mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: column as u16, row: row as u16, modifiers: KeyModifiers::NONE })
    }

    #[test]
    fn a_click_selects_an_answer_takes_it_on_the_second_and_one_beside_the_pick_closes_it() {
        let mut fixture = fixture("settings-click", "", None);
        go_to(&mut fixture.page, "task.also_noticed");
        press(&mut fixture.page, &[KeyCode::Enter]);
        let shown = screen(&mut fixture.page);
        click(&mut fixture.page, &shown, "  \"ask\"  ");
        assert!(matches!(&fixture.page.mode, Mode::Choose(chooser) if chooser.index == 2), "the first click selects");
        assert_eq!(file(&fixture, "config.toml"), "", "and writes nothing");
        let shown = screen(&mut fixture.page);
        click(&mut fixture.page, &shown, "▌\"ask\"");
        assert_eq!(file(&fixture, "config.toml"), "[task]\nalso_noticed = \"ask\"\n", "the second takes it");
        press(&mut fixture.page, &[KeyCode::Enter]);
        let shown = screen(&mut fixture.page);
        click(&mut fixture.page, &shown, "esc close");
        assert!(fixture.page.is_normal(), "a key in the hints is pressed by a click");
        press(&mut fixture.page, &[KeyCode::Enter]);
        let shown = screen(&mut fixture.page);
        click(&mut fixture.page, &shown, "Settings");
        assert!(fixture.page.is_normal(), "a click beside the pick closes it");
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
        assert!(shown.contains("│ Which skill?") && shown.contains("▌\"kebbie:work-rules\"      built in") && shown.contains(" Your skill, by name      <skill> or <plugin>:<skill>") && shown.contains("A Markdown file, by path") && shown.contains("Copy kebbie's to"), "{shown}");
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
        let mut alone = Page::new(Paths { machine: fixture.dir.join("config.toml"), repos_dir: fixture.dir.join("repos"), claude_home: fixture.dir.join("claude"), kebbie: fixture.dir.join("kebbie"), profiles: fixture.dir.join("profiles"), codex_home: fixture.dir.join("codex"), path: Vec::new(), kebbie_command: "kebbie".to_string(), sessions: Some(fixture.dir.join("state/sessions")) }, None);
        alone.load();
        press(&mut alone, &[KeyCode::Tab]);
        assert_eq!(alone.tab, Tab::Machine);
        assert!(said(&alone).contains("No repo here"));
    }

    #[test]
    fn a_file_that_does_not_parse_is_shown_and_left_alone() {
        let mut fixture = fixture("settings-broken", "[task]\nsandbox = \"yes\"\n", None);
        assert!(fixture.page.entries.is_empty());
        let shown = screen(&mut fixture.page).split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(shown.contains("task.sandbox: must be true or false"), "the error shows, wherever the path before it makes it wrap: {shown}");
        press(&mut fixture.page, &[KeyCode::Char(' '), KeyCode::Char('d')]);
        assert_eq!(file(&fixture, "config.toml"), "[task]\nsandbox = \"yes\"\n");
    }

    const CODEX_BUILD: &str = "# for small builds\nabout = \"Codex, building: a small, well-defined task done by another model family\"\nagent = \"codex\" # not Claude\nmodel = \"gpt-6.1-sol\"\neffort = \"medium\"\nrole = \"codex.md\"\n";

    fn agents_fixture(name: &str) -> Fixture {
        let fixture = fixture(name, "", None);
        let dir = &fixture.dir;
        for folder in ["profiles", "codex", "claude"] {
            std::fs::create_dir_all(dir.join(folder)).unwrap();
        }
        std::fs::write(dir.join("profiles/codex.toml"), "about = \"Codex on its own config\"\nagent = \"codex\"\neffort = \"medium\"\nrole = \"codex.md\"\n").unwrap();
        std::fs::write(dir.join("profiles/codex.md"), "Run the tests before each commit.\n").unwrap();
        std::fs::write(dir.join("profiles/designer.toml"), "about = \"UI work: mockup first\"\nmodel = \"opus\"\neffort = \"high\"\nrole = \"designer.md\"\n").unwrap();
        std::fs::write(dir.join("profiles/designer.md"), "Measure in the browser.\n").unwrap();
        std::fs::write(dir.join("kebbie/Cargo.toml"), "[package]\nname = \"kebbie\"\nversion = \"0.30.0\"\n").unwrap();
        for role in ["roles/task.md", "skills/review/SKILL.md"] {
            std::fs::create_dir_all(dir.join("kebbie").join(role).parent().unwrap()).unwrap();
            std::fs::write(dir.join("kebbie").join(role), "kebbie's own role\n").unwrap();
        }
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

    fn line_of<'a>(shown: &'a str, text: &str) -> &'a str {
        shown.lines().find(|line| line.contains(text)).unwrap_or_else(|| panic!("{text}: {shown}"))
    }

    #[test]
    fn the_profiles_section_keeps_kebbies_profiles_apart_from_yours_with_each_roles_two_layers() {
        let mut fixture = agents_fixture("settings-profiles");
        let names: Vec<String> = fixture.page.sections().into_iter().map(|(name, _)| name).collect();
        let at = names.iter().position(|name| name == PROFILES).unwrap();
        assert_eq!((names[at - 1].as_str(), names[at + 1].as_str()), ("Review & docs", "Updates"));
        go_to_profile(&mut fixture.page, "designer");
        let (shown, buffer) = screen_at(&mut fixture.page, 100, 30);
        assert!(shown.contains("▸ Profiles 6"), "{shown}");
        assert!(screen_at(&mut fixture.page, 300, 30).0.contains(&format!("writes {}", fixture.dir.join("profiles/<name>.toml").display())));
        let at = |text: &str| shown.find(text).unwrap_or_else(|| panic!("{text}: {shown}"));
        assert!(at(" FROM KEBBIE") < at(" task ") && at(" ops ") < at(" YOURS") && at(" YOURS") < at(" codex "), "kebbie's three, then the user's: {shown}");
        assert!(line_of(&shown, "FROM KEBBIE").ends_with(" FROM KEBBIE  3") && line_of(&shown, "YOURS").ends_with(" YOURS  3"), "each group headed as the board's, with its count: {shown}");
        assert!(line_of(&shown, "FROM KEBBIE").len() < line_of(&shown, "follow kebbie's updates; your settings on them are kept").len(), "{shown}");
        assert!(screen_at(&mut fixture.page, 300, 30).0.contains(&format!("{} · an update never writes here", fixture.dir.join("profiles").display())));
        assert!(line_of(&shown, " task ").ends_with("task         claude · opus                          kebbie's task role"), "{shown}");
        assert!(line_of(&shown, " reviewer ").ends_with("reviewer     claude · opus · max                    kebbie's review role"), "{shown}");
        assert!(line_of(&shown, " codex ").ends_with("codex        codex · gpt-5.1-codex-max ⚠ · medium   + codex.md"), "{shown}");
        assert!(line_of(&shown, " codex-build ").ends_with("codex-build  codex · gpt-6.1-sol · medium           + codex.md"), "a shared role shows on both: {shown}");
        assert!(line_of(&shown, " designer ").ends_with("▌ designer     claude · opus · high                   + designer.md"), "{shown}");
        assert!(line_of(&shown, "role:  ").ends_with("    role:  kebbie's task role, from kebbie 0.30.0, updates with kebbie") && line_of(&shown, "+ designer.md, ").ends_with("         + designer.md, yours: read after kebbie's role"), "{shown}");
        assert!(shown.contains("    m model  e effort  r role  n new profile  ⏎ open file  x delete"), "{shown}");
        assert!(shown.contains("  designer: claude, opus, high · role designer.md"), "the line under the page sums the selected profile up: {shown}");
        assert!(shown.lines().any(|line| line.trim_start() == "↑ ↓ profile · m model · e effort · r role · n new · tab repo · esc quit"), "{shown}");
        let (wide, _) = screen_at(&mut fixture.page, 150, 30);
        assert!(wide.lines().any(|line| line.trim_start() == "↑ ↓ profile · ← → section · m model · e effort · r role · n new · ⏎ open · d default · x delete · tab machine / repo · / search · esc quit"), "the section's keys and the page's own, as on every section: {wide}");
        assert_eq!(color_of(&buffer, " task ", "opus"), Some(MUTED), "the agent's own default shows dim");
        assert_eq!(color_of(&buffer, " reviewer ", "opus"), Some(TEXT), "the profile's own model does not");
        assert_eq!(color_of(&buffer, " codex  ", "gpt-5.1-codex-max"), Some(RED));
        assert!(shown.contains("dim = the agent's own default:") && shown.contains("Claude's from "), "{shown}");
        go_to_profile(&mut fixture.page, "codex");
        let (shown, _) = screen_at(&mut fixture.page, 100, 30);
        assert!(shown.contains("⚠ codex runs gpt-5.1-codex-max from Codex's config.toml, which Codex does not offer") && shown.contains("account; pick a model for it with m."), "the selected profile's ⚠ comes first: {shown}");
        go_to_profile(&mut fixture.page, "task");
        let (shown, _) = screen_at(&mut fixture.page, 100, 30);
        assert!(shown.contains("    m model  e effort  r role  n new profile  ⏎ open file  d default"), "kebbie's own keep d and cannot be deleted: {shown}");
        assert!(line_of(&shown, "role:  ").ends_with("kebbie's task role, from kebbie 0.30.0, updates with kebbie") && !shown.contains("yours: read after") && shown.contains("  task: claude, opus by Claude's default"), "kebbie's role alone: {shown}");
        press(&mut fixture.page, &[KeyCode::Tab]);
        assert!(!fixture.page.sections().iter().any(|(name, _)| name == PROFILES), "profiles are this machine's, never a repo's");
    }

    #[test]
    fn r_on_one_of_your_profiles_picks_a_role_file_with_who_shares_it_or_makes_one_that_overwrites_none() {
        let mut fixture = agents_fixture("settings-role-pick");
        let read = |name: &str| std::fs::read_to_string(fixture.dir.join("profiles").join(name)).unwrap_or_default();
        go_to_profile(&mut fixture.page, "designer");
        press(&mut fixture.page, &[KeyCode::Char('r')]);
        let (shown, _) = screen_at(&mut fixture.page, 100, 30);
        assert!(shown.contains("╭ role · designer ") && shown.contains("│ Which role?") && shown.contains("│ Your lines, read after kebbie's task role."), "{shown}");
        assert!(shown.contains("│ ▌designer.md         only designer · now") && shown.contains("│  codex.md            shared with codex, codex-build"), "{shown}");
        assert!(shown.contains("│  New role            designer-2.md, an empty file") && shown.contains("│  Copy of designer.md to change it for this profile alone") && shown.contains("│  None                kebbie's role alone"), "{shown}");
        assert!(shown.contains("│ A shared file changes for every profile that uses it.") && shown.contains("│ ↑ ↓ choose · ⏎ set · o open the file · esc close"), "{shown}");
        assert!(matches!(fixture.page.handle(KeyEvent::from(KeyCode::Char('o'))), Effect::Open(file) if file == fixture.dir.join("profiles/designer.md")), "o opens the selected file");
        press(&mut fixture.page, &[KeyCode::Char('r'), KeyCode::Down, KeyCode::Enter]);
        assert_eq!(read("designer.toml"), "about = \"UI work: mockup first\"\nmodel = \"opus\"\neffort = \"high\"\nrole = \"codex.md\"\n", "only the role line changes");
        assert_eq!(read("designer.md"), "Measure in the browser.\n", "the file it left stays");
        assert!(said(&fixture.page).starts_with("✓ wrote role = \"codex.md\" to profiles/designer.toml"), "{}", said(&fixture.page));
        press(&mut fixture.page, &[KeyCode::Char('r')]);
        let (shown, _) = screen_at(&mut fixture.page, 100, 30);
        assert!(shown.contains("│ ▌codex.md            shared with codex, codex-build, designer ·") && shown.contains("│                      now  ") && shown.contains("│  designer.md         used by none") && shown.contains("Copy of codex.md"), "{shown}");
        let Mode::Choose(chooser) = &fixture.page.mode else { panic!("a pick") };
        let copy = chooser.choices.iter().position(|choice| choice.label == "Copy of codex.md").unwrap();
        press(&mut fixture.page, &vec![KeyCode::Down; copy]);
        assert!(matches!(fixture.page.handle(KeyEvent::from(KeyCode::Enter)), Effect::Open(file) if file == fixture.dir.join("profiles/designer-2.md")), "a copy opens to be changed");
        assert_eq!((read("designer-2.md"), read("designer.md")), ("Run the tests before each commit.\n".to_string(), "Measure in the browser.\n".to_string()), "named after the profile, beside the file that has its name");
        assert!(read("designer.toml").contains("role = \"designer-2.md\"\n"));
        fixture.page.opened(&fixture.dir.join("profiles/designer-2.md"), Ok(true));
        assert_eq!(said(&fixture.page), "profiles/designer-2.md: read after kebbie's role by designer");
        press(&mut fixture.page, &[KeyCode::Char('r')]);
        let Mode::Choose(chooser) = &fixture.page.mode else { panic!("a pick") };
        let new = chooser.choices.iter().position(|choice| choice.label == "New role").unwrap();
        press(&mut fixture.page, &vec![KeyCode::Down; new]);
        press(&mut fixture.page, &[KeyCode::Char('o')]);
        assert!(said(&fixture.page).contains("o opens a role file; this one has none yet") && matches!(fixture.page.mode, Mode::Choose(_)), "{}", said(&fixture.page));
        assert!(matches!(fixture.page.handle(KeyEvent::from(KeyCode::Enter)), Effect::Open(file) if file == fixture.dir.join("profiles/designer-3.md")));
        assert_eq!(read("designer-3.md"), "", "a new role is an empty file");
        press(&mut fixture.page, &[KeyCode::Char('r')]);
        press(&mut fixture.page, &[KeyCode::Down; 9]);
        press(&mut fixture.page, &[KeyCode::Enter]);
        assert!(!read("designer.toml").contains("role"), "None, the last, removes the key: {}", read("designer.toml"));
        assert!(fixture.dir.join("profiles/designer-3.md").exists(), "and leaves every file");
    }

    #[test]
    fn r_on_a_kebbie_profile_is_a_menu_that_shows_kebbies_role_and_adds_your_lines_in_your_own_folder() {
        let mut fixture = agents_fixture("settings-role-menu");
        go_to_profile(&mut fixture.page, "task");
        press(&mut fixture.page, &[KeyCode::Char('r')]);
        let (shown, _) = screen_at(&mut fixture.page, 100, 30);
        assert!(shown.contains("╭ role · task ") && shown.contains("│ task is kebbie's: its role comes with kebbie and an        │") && shown.contains("│ update replaces it, so it is not edited here."), "{shown}");
        assert!(shown.contains("│ ▸ Show kebbie's role ") && shown.contains("│   roles/task.md, read-only") && shown.contains("│   Add your lines ") && shown.contains("│   profiles/task.md, read after kebbie's role by every task"), "{shown}");
        assert!(line_of(&shown, "▸ Show kebbie's role").contains(" s   │") && line_of(&shown, "  Add your lines").contains(" a   │") && shown.contains("│ ↑ ↓ choose · ⏎ run · esc close"), "the menu's keys, as the task menu shows them: {shown}");
        assert!(matches!(fixture.page.handle(KeyEvent::from(KeyCode::Enter)), Effect::Show(file) if file == fixture.dir.join("kebbie/roles/task.md")), "kebbie's role is shown, never edited");
        press(&mut fixture.page, &[KeyCode::Char('r')]);
        assert!(matches!(fixture.page.handle(KeyEvent::from(KeyCode::Char('a'))), Effect::Open(file) if file == fixture.dir.join("profiles/task.md")));
        assert_eq!(file(&fixture, "profiles/task.toml"), "role = \"task.md\"\n", "the role goes in the user's own file for task");
        assert_eq!((file(&fixture, "profiles/task.md"), file(&fixture, "kebbie/roles/task.md")), (String::new(), "kebbie's own role\n".to_string()));
        fixture.page.said = None;
        let (shown, _) = screen_at(&mut fixture.page, 120, 30);
        assert!(line_of(&shown, " task ").ends_with("kebbie's task role + task.md") && shown.contains("         + task.md, yours: read after kebbie's role"), "{shown}");
        assert!(shown.contains("task: claude, opus by Claude's default · role task.md · your settings in profiles/task.toml"), "{shown}");
        press(&mut fixture.page, &[KeyCode::Char('r')]);
        assert!(matches!(fixture.page.handle(KeyEvent::from(KeyCode::Char('a'))), Effect::Open(file) if file == fixture.dir.join("profiles/task.md")), "a second a opens the same lines");
        std::fs::write(fixture.dir.join("profiles/ops.md"), "Mine.\n").unwrap();
        std::fs::write(fixture.dir.join("profiles/helper.toml"), "role = \"ops.md\"\n").unwrap();
        fixture.page.load();
        go_to_profile(&mut fixture.page, "ops");
        press(&mut fixture.page, &[KeyCode::Char('r')]);
        assert!(screen_at(&mut fixture.page, 100, 30).0.contains("│   profiles/ops.md, shared with helper, read after kebbie's"), "a file already there says who else reads it: {}", screen_at(&mut fixture.page, 100, 30).0);
        press(&mut fixture.page, &[KeyCode::Char('a')]);
        assert_eq!((file(&fixture, "profiles/ops.toml"), file(&fixture, "profiles/ops.md")), ("role = \"ops.md\"\n".to_string(), "Mine.\n".to_string()), "a file already there is the user's and stays");
        go_to_profile(&mut fixture.page, "reviewer");
        press(&mut fixture.page, &[KeyCode::Char('r')]);
        let (shown, _) = screen_at(&mut fixture.page, 100, 30);
        assert!(!shown.contains("New profile from reviewer"), "a profile made from the reviewer would be a task's, which kebbie review never runs: {shown}");
        assert!(shown.contains("│   skills/review/SKILL.md, read-only") && shown.contains("profiles/reviewer.md, read after kebbie's role by every") && shown.contains("│ review  "), "{shown}");
        assert!(matches!(click(&mut fixture.page, &shown, "Add your lines"), Effect::Open(file) if file == fixture.dir.join("profiles/reviewer.md")), "one click runs a choice, as in the board's menus");
        press(&mut fixture.page, &[KeyCode::Char('r')]);
        let (shown, _) = screen_at(&mut fixture.page, 100, 30);
        assert!(matches!(click(&mut fixture.page, &shown, "skills/review/SKILL.md, read-only"), Effect::Show(file) if file == fixture.dir.join("kebbie/skills/review/SKILL.md")), "its second line too");
    }

    #[test]
    fn a_role_that_cannot_be_read_or_a_profile_that_cannot_be_written_is_said_and_leaves_no_file() {
        let mut fixture = agents_fixture("settings-role-errors");
        let profiles = fixture.dir.join("profiles");
        go_to_profile(&mut fixture.page, "designer");
        press(&mut fixture.page, &[KeyCode::Char('r')]);
        let Mode::Choose(chooser) = &fixture.page.mode else { panic!("a pick") };
        let copy = chooser.choices.iter().position(|choice| choice.label == "Copy of designer.md").unwrap();
        std::fs::remove_file(profiles.join("designer.md")).unwrap();
        std::fs::create_dir(profiles.join("designer.md")).unwrap();
        press(&mut fixture.page, &vec![KeyCode::Down; copy]);
        assert!(matches!(fixture.page.handle(KeyEvent::from(KeyCode::Enter)), Effect::Handled));
        assert!(said(&fixture.page).starts_with("✗ designer.md: ") && !profiles.join("designer-2.md").exists(), "a copy it cannot read is said, never an empty file: {}", said(&fixture.page));
        let locked = fixture.dir.join("locked");
        std::fs::create_dir_all(&locked).unwrap();
        std::fs::write(locked.join("task.toml"), "effort = \"high\"\n").unwrap();
        std::os::unix::fs::symlink(locked.join("task.toml"), profiles.join("task.toml")).unwrap();
        std::fs::set_permissions(&locked, std::os::unix::fs::PermissionsExt::from_mode(0o555)).unwrap();
        fixture.page.load();
        go_to_profile(&mut fixture.page, "task");
        press(&mut fixture.page, &[KeyCode::Char('r')]);
        let effect = fixture.page.handle(KeyEvent::from(KeyCode::Char('a')));
        std::fs::set_permissions(&locked, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        assert!(matches!(effect, Effect::Handled) && said(&fixture.page).starts_with("✗ ") && !profiles.join("task.md").exists(), "a role whose profile cannot be written goes again: {}", said(&fixture.page));
    }

    fn wizard(page: &Page) -> &Wizard {
        match &page.mode {
            Mode::Wizard(wizard) => wizard,
            _ => panic!("the wizard is not open"),
        }
    }

    fn files_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir).unwrap().flatten().map(|entry| entry.file_name().to_string_lossy().to_string()).collect();
        names.sort();
        names
    }

    fn fake_codex(fixture: &Fixture) {
        let bin = fixture.dir.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        crate::executable::write(bin.join("codex"), "#!/bin/sh\necho codex-cli 0.163.0\n").unwrap();
    }

    #[test]
    fn n_walks_six_steps_to_a_profile_that_loads_with_its_role_and_never_a_copy_of_kebbies_role() {
        let mut fixture = agents_fixture("settings-wizard");
        fake_codex(&fixture);
        let profiles = fixture.dir.join("profiles");
        let before = files_in(&profiles);
        go_to_profile(&mut fixture.page, "designer");
        press(&mut fixture.page, &[KeyCode::Char('n')]);
        let (shown, _) = screen_at(&mut fixture.page, 100, 34);
        assert!(shown.contains("╭ new profile ") && shown.contains("│ step 1 of 6") && shown.contains("│ Its name?") && shown.contains("│ ⏎ next · esc cancel"), "{shown}");
        type_text(&mut fixture.page, "task");
        press(&mut fixture.page, &[KeyCode::Enter]);
        assert!(wizard(&fixture.page).step == 0 && screen_at(&mut fixture.page, 100, 34).0.contains("task is kebbie's own profile"), "a built-in's name is refused");
        press(&mut fixture.page, &[KeyCode::Backspace; 4]);
        type_text(&mut fixture.page, "codex");
        press(&mut fixture.page, &[KeyCode::Enter]);
        assert!(wizard(&fixture.page).step == 0 && screen_at(&mut fixture.page, 100, 34).0.contains("codex is a profile already"), "an existing name is refused");
        type_text(&mut fixture.page, "-Big");
        assert!(screen_at(&mut fixture.page, 100, 34).0.contains("lower-case letters, digits and dashes, starting with a letter"));
        press(&mut fixture.page, &[KeyCode::Backspace; 9]);
        type_text(&mut fixture.page, "designer-codex");
        press(&mut fixture.page, &[KeyCode::Enter]);
        let (shown, _) = screen_at(&mut fixture.page, 100, 34);
        assert!(shown.contains("│ step 2 of 6") && shown.contains("│ Start from which profile?") && shown.contains("│ ▌designer ") && shown.contains("│  ops ") && !shown.contains("│  reviewer"), "it starts from the selected profile, never the reviewer: {shown}");
        press(&mut fixture.page, &[KeyCode::Enter]);
        press(&mut fixture.page, &[KeyCode::Down]);
        let (shown, _) = screen_at(&mut fixture.page, 100, 34);
        assert!(shown.contains("│ step 3 of 6") && shown.contains("│ Which agent?") && shown.contains("│ It runs the task; the model comes next."), "{shown}");
        assert!(shown.contains("│  Claude Code         kebbie's own default") && shown.contains("│ ▌Codex               codex-cli 0.163.0 · kebbie's plugin not ") && shown.contains("│                      installed: kebbie setup codex"), "Codex's version and kebbie's plugin, as kebbie profiles says them: {shown}");
        assert!(shown.contains("│ ↑ ↓ choose · ⏎ next · ← back · esc cancel"), "{shown}");
        press(&mut fixture.page, &[KeyCode::Enter]);
        assert!(screen_at(&mut fixture.page, 100, 34).0.contains("│ ▌Codex's default     gpt-5.1-codex-max, not offered to you ⚠"), "the same answers as m, with no now");
        press(&mut fixture.page, &[KeyCode::Up; 9]);
        press(&mut fixture.page, &[KeyCode::Enter]);
        let (shown, _) = screen_at(&mut fixture.page, 100, 34);
        assert!(shown.contains("│ step 5 of 6") && shown.contains("│ How much effort?") && shown.contains("│ The levels GPT-6.1-Sol takes, from Codex's list."), "{shown}");
        let high = wizard(&fixture.page).efforts.1.iter().position(|answer| answer.then == Pick::Value(Some("high".to_string()))).unwrap();
        press(&mut fixture.page, &[KeyCode::Up; 9]);
        press(&mut fixture.page, &vec![KeyCode::Down; high]);
        press(&mut fixture.page, &[KeyCode::Enter]);
        let (shown, _) = screen_at(&mut fixture.page, 100, 34);
        assert!(shown.contains("│ step 6 of 6") && shown.contains("│ Your lines, read after kebbie's task role, which updates with") && shown.contains("│ kebbie."), "{shown}");
        assert!(shown.contains("│ ▌New role            designer-codex.md, empty, yours to write") && shown.contains("│  Copy of designer.md the same lines, changed for this one alone"), "{shown}");
        assert!(shown.contains("│  codex.md            shared with codex, codex-build") && shown.contains("│  designer.md         shared with designer") && shown.contains("│  None                kebbie's role alone"), "{shown}");
        press(&mut fixture.page, &[KeyCode::Down, KeyCode::Enter]);
        let (shown, _) = screen_at(&mut fixture.page, 100, 34);
        assert!(shown.contains("│ check") && shown.contains("│ Add to ") && shown.contains("│ designer-codex.toml") && shown.contains("│ designer-codex.md  a copy of designer.md"), "{shown}");
        assert!(shown.contains("│ about = \"UI work: mockup first\"\n") || shown.contains("│ about = \"UI work: mockup first\""), "{shown}");
        assert!(shown.contains("│ agent = \"codex\"") && shown.contains("│ model = \"gpt-6.1-sol\"") && shown.contains("│ effort = \"high\"") && shown.contains("│ role = \"designer-codex.md\""), "{shown}");
        assert!(shown.contains("│ y add · g add and open the role · ← back · esc cancel"), "{shown}");
        assert_eq!(files_in(&profiles), before, "nothing is written before y or g");
        let effect = fixture.page.handle(KeyEvent::from(KeyCode::Char('g')));
        assert!(matches!(effect, Effect::Open(file) if file == profiles.join("designer-codex.md")), "g opens the role");
        assert_eq!(file(&fixture, "profiles/designer-codex.toml"), "about = \"UI work: mockup first\"\nagent = \"codex\"\nmodel = \"gpt-6.1-sol\"\neffort = \"high\"\nrole = \"designer-codex.md\"\n");
        assert_eq!(file(&fixture, "profiles/designer-codex.md"), "Measure in the browser.\n", "a copy of the user's role, never kebbie's");
        assert!(files_in(&profiles).iter().all(|name| !file(&fixture, &format!("profiles/{name}")).contains("kebbie's own role")), "kebbie's role text is in no file of the user's");
        let made = profiles::load_in(&profiles, "designer-codex").unwrap();
        assert_eq!((made.agent, made.model.as_deref(), made.effort.as_deref()), (Agent::Codex, Some("gpt-6.1-sol"), Some("high")));
        assert_eq!(fixture.page.profile().map(|profile| profile.name), Some("designer-codex".to_string()), "the new row is selected");
        assert!(said(&fixture.page).starts_with("✓ added profiles/designer-codex.toml, its role designer-codex.md"), "{}", said(&fixture.page));
    }

    #[test]
    fn a_codex_profile_from_one_with_claudes_rules_leaves_them_out_and_esc_anywhere_writes_nothing() {
        let mut fixture = agents_fixture("settings-wizard-codex");
        let profiles = fixture.dir.join("profiles");
        std::fs::write(profiles.join("careful.toml"), "for = \"risky changes\"\npermission_mode = \"plan\"\ndeny = [\"Bash(rm:*)\"]\n").unwrap();
        fixture.page.load();
        let before = files_in(&profiles);
        go_to_profile(&mut fixture.page, "careful");
        press(&mut fixture.page, &[KeyCode::Char('n')]);
        type_text(&mut fixture.page, "careful-codex");
        press(&mut fixture.page, &[KeyCode::Enter, KeyCode::Enter, KeyCode::Down, KeyCode::Enter, KeyCode::Enter, KeyCode::Enter]);
        let roles = wizard(&fixture.page).roles();
        press(&mut fixture.page, &vec![KeyCode::Down; roles.len() - 1]);
        press(&mut fixture.page, &[KeyCode::Enter]);
        let (shown, _) = screen_at(&mut fixture.page, 100, 34);
        assert!(shown.contains("│ permission_mode = \"plan\"") && !shown.contains("│ deny =") && shown.contains("│ Left out: deny, which only Claude Code reads."), "{shown}");
        assert!(shown.contains("│ y add · g add and open the file"), "with no role, g opens the profile's file: {shown}");
        press(&mut fixture.page, &[KeyCode::Left]);
        assert_eq!(wizard(&fixture.page).step, 5, "← goes back a step");
        press(&mut fixture.page, &[KeyCode::Esc]);
        assert!(fixture.page.is_normal());
        assert_eq!(files_in(&profiles), before, "esc writes nothing");
        go_to_profile(&mut fixture.page, "careful");
        press(&mut fixture.page, &[KeyCode::Char('n')]);
        type_text(&mut fixture.page, "careful-codex");
        press(&mut fixture.page, &[KeyCode::Enter, KeyCode::Enter]);
        let (shown, _) = screen_at(&mut fixture.page, 100, 34);
        click(&mut fixture.page, &shown, " Codex   ");
        assert_eq!((wizard(&fixture.page).step, wizard(&fixture.page).agent()), (2, Agent::Codex), "a click selects an answer");
        let (shown, _) = screen_at(&mut fixture.page, 100, 34);
        click(&mut fixture.page, &shown, "▌Codex");
        assert_eq!(wizard(&fixture.page).step, 3, "and a second goes on");
        let (shown, _) = screen_at(&mut fixture.page, 100, 34);
        click(&mut fixture.page, &shown, "Settings");
        assert_eq!(wizard(&fixture.page).step, 3, "a click beside the steps leaves them open");
        press(&mut fixture.page, &[KeyCode::Enter, KeyCode::Enter]);
        press(&mut fixture.page, &vec![KeyCode::Down; roles.len() - 1]);
        press(&mut fixture.page, &[KeyCode::Enter, KeyCode::Char('y')]);
        assert_eq!(file(&fixture, "profiles/careful-codex.toml"), "for = \"risky changes\"\nagent = \"codex\"\npermission_mode = \"plan\"\n");
        assert!(profiles::load_in(&profiles, "careful-codex").is_ok());
        go_to_profile(&mut fixture.page, "task");
        press(&mut fixture.page, &[KeyCode::Char('r'), KeyCode::Char('n')]);
        assert_eq!(wizard(&fixture.page).from().map(|profile| profile.name.as_str()), Some("task"), "n in task's role menu starts from task");
        type_text(&mut fixture.page, "mine");
        press(&mut fixture.page, &[KeyCode::Enter; 6]);
        press(&mut fixture.page, &[KeyCode::Char('y')]);
        assert_eq!((file(&fixture, "profiles/mine.md"), file(&fixture, "profiles/mine.toml").contains("role = \"mine.md\"")), (String::new(), true), "a new role starts empty: kebbie's lines come from kebbie, read first");
    }

    #[test]
    fn the_wizard_follows_the_start_it_was_given_and_a_click_while_typing_a_model_takes_that_answer() {
        let mut fixture = agents_fixture("settings-wizard-follow");
        go_to_profile(&mut fixture.page, "designer");
        press(&mut fixture.page, &[KeyCode::Char('n')]);
        type_text(&mut fixture.page, "follow");
        press(&mut fixture.page, &[KeyCode::Enter]);
        let at = wizard(&fixture.page).starts.iter().position(|profile| profile.name == "codex-build").unwrap();
        let from = wizard(&fixture.page).start;
        press(&mut fixture.page, &vec![if at > from { KeyCode::Down } else { KeyCode::Up }; at.abs_diff(from)]);
        press(&mut fixture.page, &[KeyCode::Enter]);
        assert_eq!(wizard(&fixture.page).agent(), Agent::Codex, "the agent step opens on the chosen start's agent");
        press(&mut fixture.page, &[KeyCode::Left]);
        let designer = wizard(&fixture.page).starts.iter().position(|profile| profile.name == "designer").unwrap();
        press(&mut fixture.page, &vec![KeyCode::Down; designer.saturating_sub(at)]);
        press(&mut fixture.page, &[KeyCode::Enter]);
        assert_eq!(wizard(&fixture.page).agent(), Agent::Claude, "and follows a start chosen again");
        press(&mut fixture.page, &[KeyCode::Enter]);
        let typed = wizard(&fixture.page).models.1.iter().position(|answer| answer.then == Pick::Type).unwrap();
        press(&mut fixture.page, &[KeyCode::Up; 9]);
        press(&mut fixture.page, &vec![KeyCode::Down; typed]);
        press(&mut fixture.page, &[KeyCode::Enter]);
        type_text(&mut fixture.page, "claude-half");
        let (shown, _) = screen_at(&mut fixture.page, 100, 34);
        click(&mut fixture.page, &shown, "the newest Sonnet");
        let shown_after = screen_at(&mut fixture.page, 100, 34).0;
        assert!(wizard(&fixture.page).typing.is_none() && wizard(&fixture.page).step == 3 && wizard(&fixture.page).model() == Some("sonnet".to_string()), "the click leaves the typing and selects that answer, nothing half-typed taken: {shown_after}");
        assert!(!shown_after.contains("claude-half"), "{shown_after}");
    }

    #[test]
    fn y_on_the_wizard_keeps_a_role_file_that_appeared_meanwhile_and_says_a_copy_it_cannot_read() {
        let mut fixture = agents_fixture("settings-wizard-guard");
        let profiles = fixture.dir.join("profiles");
        go_to_profile(&mut fixture.page, "designer");
        press(&mut fixture.page, &[KeyCode::Char('n')]);
        type_text(&mut fixture.page, "guard");
        for _ in 0..6 {
            press(&mut fixture.page, &[KeyCode::Enter]);
            assert!(matches!(fixture.page.mode, Mode::Wizard(_)), "{}", screen_at(&mut fixture.page, 100, 34).0);
        }
        assert_eq!(wizard(&fixture.page).chosen_role(), Role::New("guard.md".to_string()));
        std::fs::write(profiles.join("guard.md"), "Someone's lines.\n").unwrap();
        press(&mut fixture.page, &[KeyCode::Char('y')]);
        assert_eq!(wizard(&fixture.page).error.as_deref(), Some("guard.md is in the folder already"));
        assert_eq!((file(&fixture, "profiles/guard.md"), profiles.join("guard.toml").exists()), ("Someone's lines.\n".to_string(), false), "a file this call did not write stays");
        std::fs::remove_file(profiles.join("guard.md")).unwrap();
        press(&mut fixture.page, &[KeyCode::Left, KeyCode::Down, KeyCode::Enter]);
        assert!(matches!(wizard(&fixture.page).chosen_role(), Role::Copy { .. }));
        std::fs::remove_file(profiles.join("designer.md")).unwrap();
        std::fs::create_dir(profiles.join("designer.md")).unwrap();
        press(&mut fixture.page, &[KeyCode::Char('y')]);
        assert!(wizard(&fixture.page).error.as_deref().is_some_and(|error| error.starts_with("designer.md: ")), "{:?}", wizard(&fixture.page).error);
        assert!(!profiles.join("guard.md").exists() && !profiles.join("guard.toml").exists(), "a copy it cannot read writes nothing");
    }

    #[test]
    fn x_deletes_your_profile_and_its_role_only_when_no_other_reads_it_and_never_kebbies_or_an_open_tasks() {
        let mut fixture = agents_fixture("settings-delete");
        let exists = |fixture: &Fixture, name: &str| fixture.dir.join("profiles").join(name).exists();
        go_to_profile(&mut fixture.page, "task");
        press(&mut fixture.page, &[KeyCode::Char('x')]);
        assert!(fixture.page.is_normal() && said(&fixture.page).starts_with("✗ task is kebbie's and stays"), "{}", said(&fixture.page));
        go_to_profile(&mut fixture.page, "codex");
        press(&mut fixture.page, &[KeyCode::Char('x')]);
        let (shown, _) = screen_at(&mut fixture.page, 100, 30);
        assert!(shown.contains("╭ delete ") && shown.contains("│ codex ") && shown.contains("│ profiles/codex.toml goes; its role codex.md stays, as codex-build") && shown.contains("│ Delete this profile?  y / n"), "{shown}");
        press(&mut fixture.page, &[KeyCode::Char('z')]);
        assert!(screen_at(&mut fixture.page, 100, 30).0.contains("press y or n") && exists(&fixture, "codex.toml"), "another key asks again");
        press(&mut fixture.page, &[KeyCode::Char('y')]);
        assert!(!exists(&fixture, "codex.toml") && exists(&fixture, "codex.md"), "a role another profile reads stays");
        assert_eq!(said(&fixture.page), "✓ removed profiles/codex.toml; its role codex.md stays for codex-build");
        std::fs::create_dir_all(fixture.dir.join("state/sessions")).unwrap();
        std::fs::write(fixture.dir.join("state/sessions/billing@app"), "{\"session\":\"s\",\"mode\":null,\"profile\":\"designer\",\"agent\":\"claude\"}").unwrap();
        std::fs::write(fixture.dir.join("state/sessions/billing@app.prompt"), "designer").unwrap();
        go_to_profile(&mut fixture.page, "designer");
        press(&mut fixture.page, &[KeyCode::Char('x')]);
        assert!(fixture.page.is_normal() && said(&fixture.page).starts_with("✗ designer is the profile of billing@app, still open"), "{}", said(&fixture.page));
        std::fs::remove_file(fixture.dir.join("state/sessions/billing@app")).unwrap();
        press(&mut fixture.page, &[KeyCode::Char('x')]);
        let (shown, _) = screen_at(&mut fixture.page, 100, 30);
        assert!(shown.contains("│ profiles/designer.toml goes, and its role designer.md, which no"), "{shown}");
        press(&mut fixture.page, &[KeyCode::Char('n')]);
        assert!(fixture.page.is_normal() && exists(&fixture, "designer.toml"), "n keeps it");
        std::fs::write(fixture.dir.join("profiles/broken.toml"), "agent = \"gemini\"\nrole = \"designer.md\"\n").unwrap();
        fixture.page.load();
        go_to_profile(&mut fixture.page, "designer");
        press(&mut fixture.page, &[KeyCode::Char('x')]);
        assert!(screen_at(&mut fixture.page, 100, 30).0.contains("stays, as broken"), "a file that does not load still reads its role: {}", screen_at(&mut fixture.page, 100, 30).0);
        press(&mut fixture.page, &[KeyCode::Char('y')]);
        assert!(!exists(&fixture, "designer.toml") && exists(&fixture, "designer.md") && said(&fixture.page) == "✓ removed profiles/designer.toml; its role designer.md stays for broken", "{}", said(&fixture.page));
        std::fs::remove_file(fixture.dir.join("profiles/broken.toml")).unwrap();
        std::fs::write(fixture.dir.join("profiles/designer.toml"), "role = \"designer.md\"\n").unwrap();
        fixture.page.load();
        go_to_profile(&mut fixture.page, "designer");
        press(&mut fixture.page, &[KeyCode::Char('x'), KeyCode::Char('y')]);
        assert!(!exists(&fixture, "designer.toml") && !exists(&fixture, "designer.md"), "a role no other profile reads goes with it");
        assert_eq!(said(&fixture.page), "✓ removed profiles/designer.toml and its role designer.md");
        std::fs::write(fixture.dir.join("config.toml"), "[task]\nprofile = \"codex-build\"\n").unwrap();
        fixture.page.load();
        go_to_profile(&mut fixture.page, "codex-build");
        press(&mut fixture.page, &[KeyCode::Char('x')]);
        assert!(said(&fixture.page).starts_with("✗ task.profile names codex-build in ") && exists(&fixture, "codex-build.toml"), "{}", said(&fixture.page));
        std::fs::write(fixture.dir.join("config.toml"), "").unwrap();
        std::fs::write(fixture.dir.join("repos/app.toml"), "[task]\nprofile = \"codex-build\"\n").unwrap();
        fixture.page.load();
        press(&mut fixture.page, &[KeyCode::Char('x')]);
        assert!(said(&fixture.page).starts_with(&format!("✗ task.profile names codex-build in {}", fixture.dir.join("repos/app.toml").display())) && exists(&fixture, "codex-build.toml"), "a repo config's task.profile keeps it too: {}", said(&fixture.page));
        std::fs::write(fixture.dir.join("repos/app.toml"), "").unwrap();
        std::fs::write(fixture.dir.join("config.toml"), "[[routines]]\nid = \"nightly\"\ntitle = \"Nightly check\"\nrepo = \"app\"\nat = \"07:00\"\nprompt = \"Check it\"\nprofile = \"codex-build\"\n").unwrap();
        fixture.page.load();
        press(&mut fixture.page, &[KeyCode::Char('x')]);
        assert!(said(&fixture.page).starts_with("✗ codex-build is the profile the routine \"Nightly check\" starts its tasks with") && exists(&fixture, "codex-build.toml"), "{}", said(&fixture.page));
    }

    #[test]
    fn m_on_a_codex_profile_picks_from_codexs_own_list_and_writes_model_alone() {
        let mut fixture = agents_fixture("settings-codex-model");
        go_to_profile(&mut fixture.page, "codex-build");
        press(&mut fixture.page, &[KeyCode::Char('m')]);
        let (shown, _) = screen_at(&mut fixture.page, 100, 30);
        assert!(shown.contains("╭ model · codex-build ") && shown.contains("│ Which model?") && shown.contains("│ Codex's own list for your account."), "{shown}");
        assert!(shown.contains("│ ▌GPT-6.1-Sol         gpt-6.1-sol · now") && shown.contains("│  GPT-6-Astra         gpt-6-astra") && shown.contains("│  GPT-6.1 Terra       gpt-6.1-terra"), "{shown}");
        assert!(shown.contains("│  Codex's default     gpt-5.1-codex-max, not offered to you ⚠") && shown.contains("│  Type a model…       any id Codex accepts"), "{shown}");
        assert!(shown.contains("│ A task already running keeps its model; the next start takes it.") && shown.contains("│ ↑ ↓ choose · ⏎ set · esc close"), "{shown}");
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
        assert!(screen_at(&mut fixture.page, 100, 30).0.contains("gpt-7-preview ⚠"), "an id Codex does not list is warned about");
        press(&mut fixture.page, &[KeyCode::Char('m')]);
        assert!(screen_at(&mut fixture.page, 100, 30).0.contains("│ ▌gpt-7-preview       not offered to you ⚠ · now"), "the model in effect is an answer of its own: {}", screen_at(&mut fixture.page, 100, 30).0);
        press(&mut fixture.page, &[KeyCode::Enter]);
        assert!(file(&fixture, "profiles/codex-build.toml").contains("\nmodel = \"gpt-7-preview\"\n") && said(&fixture.page).starts_with("codex-build has that model already"), "⏎ keeps it: {}", said(&fixture.page));
        press(&mut fixture.page, &[KeyCode::Char('m'), KeyCode::Down, KeyCode::Enter]);
        assert!(!file(&fixture, "profiles/codex-build.toml").contains("model ="), "Codex's default removes the key");
        assert!(said(&fixture.page).starts_with("✓ removed model from profiles/codex-build.toml · codex-build runs codex on gpt-5.1-codex-max by Codex's default"), "{}", said(&fixture.page));
        std::fs::remove_file(fixture.dir.join("codex/models_cache.json")).unwrap();
        fixture.page.load();
        press(&mut fixture.page, &[KeyCode::Char('m')]);
        let (shown, _) = screen_at(&mut fixture.page, 100, 30);
        assert!(shown.contains("No list from Codex at ") && shown.contains("Codex's default     gpt-5.1-codex-max") && shown.contains("Type a model…") && !shown.contains("GPT-6"), "{shown}");
        assert!(!shown.contains("not offered"), "without Codex's list nothing is called not offered: {shown}");
    }

    #[test]
    fn a_pick_in_a_narrow_window_wraps_each_about_under_its_own_column_and_a_click_takes_the_row_it_hits() {
        let mut fixture = agents_fixture("settings-narrow-pick");
        go_to_profile(&mut fixture.page, "codex-build");
        press(&mut fixture.page, &[KeyCode::Char('m')]);
        let (shown, _) = screen_at(&mut fixture.page, 60, 40);
        let rows: Vec<&str> = shown.lines().collect();
        let at = rows.iter().position(|line| line.contains("│  Codex's default ")).unwrap_or_else(|| panic!("{shown}"));
        assert!(rows[at + 1].starts_with(&format!("{}│ {}", " ".repeat(rows[at].find('│').unwrap()), " ".repeat(21))) && rows[at + 1].contains("⚠"), "the about goes on under its column, not from the edge: {shown}");
        assert!(rows.iter().all(|line| line.chars().count() <= 60), "{shown}");
        click(&mut fixture.page, &shown, "GPT-6.1 Terra");
        assert!(matches!(&fixture.page.mode, Mode::Choose(chooser) if chooser.choices[chooser.index].label == "GPT-6.1 Terra"), "a click takes the row drawn there");
        let (wide, _) = screen_at(&mut fixture.page, 100, 40);
        assert!(wide.contains("│  Codex's default     gpt-5.1-codex-max, not offered to you ⚠"), "{wide}");
    }

    #[test]
    fn e_offers_the_levels_codex_lists_for_the_profiles_model_ultra_among_them() {
        let mut fixture = agents_fixture("settings-codex-effort");
        go_to_profile(&mut fixture.page, "codex-build");
        press(&mut fixture.page, &[KeyCode::Char('e')]);
        let (shown, _) = screen_at(&mut fixture.page, 100, 30);
        assert!(shown.contains("╭ effort · codex-build ") && shown.contains("│ How much effort?") && shown.contains("│ The levels GPT-6.1-Sol takes, from Codex's list."), "{shown}");
        assert!(shown.contains("│  low                 Codex's default for this model") && shown.contains("│ ▌medium              now") && shown.contains("│  ultra               only some Codex models"), "{shown}");
        assert!(shown.contains("│  Default             leave it to Codex: low"), "{shown}");
        press(&mut fixture.page, &[KeyCode::Down, KeyCode::Down, KeyCode::Down, KeyCode::Down, KeyCode::Enter]);
        assert_eq!(file(&fixture, "profiles/codex-build.toml"), CODEX_BUILD.replace("effort = \"medium\"", "effort = \"ultra\""));
        assert_eq!(fixture.page.profile().unwrap().effort.as_deref(), Some("ultra"));
        go_to_profile(&mut fixture.page, "codex");
        press(&mut fixture.page, &[KeyCode::Char('e')]);
        let (shown, _) = screen_at(&mut fixture.page, 100, 30);
        assert!(shown.contains("╭ effort · codex ") && shown.contains("The levels gpt-5.1-codex-max takes"), "a hidden model's own levels: {shown}");
        press(&mut fixture.page, &[KeyCode::Down, KeyCode::Enter]);
        assert!(!file(&fixture, "profiles/codex.toml").contains("effort"), "Default leaves it to Codex");
    }

    #[test]
    fn m_on_a_built_in_claude_profile_writes_its_own_file_and_d_takes_it_away_again() {
        let mut fixture = agents_fixture("settings-claude-model");
        go_to_profile(&mut fixture.page, "task");
        assert!(matches!(fixture.page.handle(KeyEvent::from(KeyCode::Enter)), Effect::Handled));
        assert!(said(&fixture.page).contains("task is kebbie's and has no file of yours until you set something: m, e or r writes profiles/task.toml"), "{}", said(&fixture.page));
        press(&mut fixture.page, &[KeyCode::Char('m')]);
        let (shown, _) = screen_at(&mut fixture.page, 100, 30);
        assert!(shown.contains("╭ model · task ") && shown.contains("│ Claude Code's model names."), "{shown}");
        assert!(matches!(&fixture.page.mode, Mode::Choose(chooser) if chooser.choices[0].about == format!("opus, from {}", fixture.dir.join("claude/settings.json").display()) && chooser.choices[0].now));
        assert!(shown.contains("│ ▌Claude's default    opus, from") && shown.contains("│  opus                the newest Opus") && shown.contains("│  fable               the newest Fable") && shown.contains("│  Type a model…       a full id, as claude-opus-5-5"), "{shown}");
        assert!(shown.contains("│ task is kebbie's: a model you set goes in profiles/task.toml,") && shown.contains("│ which an update never writes, and kebbie's role and rules stay") && shown.contains("│ as they ship. d on the row removes that file again.") && shown.contains("│ A task already running keeps its model; the next start takes it."), "{shown}");
        press(&mut fixture.page, &[KeyCode::Down, KeyCode::Down, KeyCode::Enter]);
        assert_eq!(file(&fixture, "profiles/task.toml"), "model = \"sonnet\"\n");
        let (shown, _) = screen_at(&mut fixture.page, 100, 30);
        assert!(line_of(&shown, " task ").contains("claude · sonnet"), "{shown}");
        fixture.page.said = None;
        assert!(screen_at(&mut fixture.page, 100, 30).0.contains("  task: claude, sonnet · your settings in profiles/task.toml"), "{shown}");
        press(&mut fixture.page, &[KeyCode::Char('e')]);
        let (shown, _) = screen_at(&mut fixture.page, 100, 30);
        assert!(shown.contains("╭ effort · task ") && shown.contains("Claude Code's effort levels, for sonnet.") && !shown.contains("ultra") && shown.contains("leave it to Claude"), "{shown}");
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
        assert_eq!(file(&fixture, "profiles/codex-build.toml"), "# for small builds\nabout = \"Codex, building: a small, well-defined task done by another model family\"\nagent = \"codex\" # not Claude\nrole = \"codex.md\"\n", "a file profile keeps everything but the two keys");
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
        assert!(line_of(&shown, " reviewer ").contains("claude · sonnet · max"), "the model kebbie review runs: {shown}");
        assert!(shown.contains("  reviewer: claude, sonnet from review.model (machine), max · your settings in\n  profiles/reviewer.toml"), "and where it comes from: {shown}");
        press(&mut fixture.page, &[KeyCode::Char('m')]);
        let (shown, _) = screen_at(&mut fixture.page, 100, 30);
        assert!(shown.contains("Claude Code's model names; this sets review.model") && shown.contains("│  The profile's       opus, from reviewer.toml") && shown.contains("│ ▌sonnet              the newest Sonnet · now"), "{shown}");
        assert!(shown.contains("This writes review.model in "), "{shown}");
        press(&mut fixture.page, &[KeyCode::Down, KeyCode::Enter]);
        assert_eq!(file(&fixture, "config.toml"), "# mine\n[review]\nmodel = \"haiku\"\n");
        assert_eq!(file(&fixture, "profiles/reviewer.toml"), "model = \"opus\"\neffort = \"max\"\n", "the reviewer's model has one place");
        assert!(said(&fixture.page).starts_with("✓ wrote review.model = \"haiku\" to config.toml"), "{}", said(&fixture.page));
        press(&mut fixture.page, &[KeyCode::Char('m'), KeyCode::Up, KeyCode::Up, KeyCode::Up, KeyCode::Enter]);
        assert!(!file(&fixture, "config.toml").contains("model ="), "the default removes review.model: {}", file(&fixture, "config.toml"));
        let (shown, _) = screen_at(&mut fixture.page, 100, 30);
        assert!(line_of(&shown, " reviewer ").contains("claude · opus · max") && said(&fixture.page).starts_with("✓ removed review.model"), "{shown}");
        press(&mut fixture.page, &[KeyCode::Char('e'), KeyCode::Up, KeyCode::Enter]);
        assert_eq!(file(&fixture, "profiles/reviewer.toml"), "model = \"opus\"\neffort = \"xhigh\"\n", "e keeps editing the profile's effort");
        assert!(!file(&fixture, "config.toml").contains("effort"));
        std::fs::write(fixture.dir.join("config.toml"), "[review]\nmodel = \"sonnet\"\n").unwrap();
        fixture.page.load();
        press(&mut fixture.page, &[KeyCode::Char('d')]);
        assert_eq!((file(&fixture, "config.toml"), fixture.dir.join("profiles/reviewer.toml").exists()), ("[review]\n".to_string(), false), "d takes the reviewer back to its defaults, review.model with them");
        assert!(said(&fixture.page).starts_with("✓ removed review.model from config.toml") && said(&fixture.page).contains("✓ removed profiles/reviewer.toml"), "{}", said(&fixture.page));
    }

    #[test]
    fn a_value_in_effect_that_no_answer_offers_is_one_of_its_own_and_enter_keeps_it() {
        let mut fixture = agents_fixture("settings-keep-current");
        let profiles = fixture.dir.join("profiles");
        std::fs::write(profiles.join("codex-build.toml"), CODEX_BUILD.replace("effort = \"medium\"", "effort = \"persistent\"")).unwrap();
        fixture.page.load();
        go_to_profile(&mut fixture.page, "codex-build");
        press(&mut fixture.page, &[KeyCode::Char('e')]);
        let (shown, _) = screen_at(&mut fixture.page, 100, 30);
        assert!(shown.contains("│ ▌persistent          not a level this model lists · now"), "{shown}");
        press(&mut fixture.page, &[KeyCode::Enter]);
        assert!(file(&fixture, "profiles/codex-build.toml").contains("effort = \"persistent\"") && said(&fixture.page).starts_with("codex-build has that effort already"), "⏎ never rewrites it to the first level: {}", said(&fixture.page));
        std::fs::write(profiles.join("broken.toml"), "agent = \"gemini\"\n").unwrap();
        std::fs::write(fixture.dir.join("config.toml"), "[task]\nprofile = \"broken\"\n").unwrap();
        fixture.page.load();
        go_to(&mut fixture.page, "task.profile");
        press(&mut fixture.page, &[KeyCode::Enter]);
        assert!(screen(&mut fixture.page).contains("│ ▌\"broken\"            no profile loads by this name · now"), "{}", screen(&mut fixture.page));
        press(&mut fixture.page, &[KeyCode::Enter]);
        assert_eq!(file(&fixture, "config.toml"), "[task]\nprofile = \"broken\"\n", "⏎ keeps the name, never none");
        std::fs::write(profiles.join("odd.toml"), "agent = \"codex\"\nmodel = \"gpt-7-preview\"\neffort = \"persistent\"\n").unwrap();
        fixture.page.load();
        go_to_profile(&mut fixture.page, "odd");
        press(&mut fixture.page, &[KeyCode::Char('n')]);
        type_text(&mut fixture.page, "odd-copy");
        press(&mut fixture.page, &[KeyCode::Enter; 3]);
        assert!(screen_at(&mut fixture.page, 100, 34).0.contains("│ ▌gpt-7-preview       not offered to you ⚠"), "the start's model is offered as it is: {}", screen_at(&mut fixture.page, 100, 34).0);
        press(&mut fixture.page, &[KeyCode::Enter, KeyCode::Enter]);
        assert_eq!((wizard(&fixture.page).model().as_deref(), wizard(&fixture.page).effort().as_deref()), (Some("gpt-7-preview"), Some("persistent")), "the wizard keeps the start's model and effort on ⏎");
    }

    #[test]
    fn task_profile_is_picked_from_the_profiles_there_are_and_none_removes_it() {
        let mut fixture = agents_fixture("settings-task-profile");
        go_to(&mut fixture.page, "task.profile");
        assert!(fixture.page.hint_lines().0.to_string().contains("⏎ pick"));
        press(&mut fixture.page, &[KeyCode::Enter]);
        let shown = screen(&mut fixture.page);
        assert!(shown.contains("│ Which profile?") && shown.contains("│ ▌none (task)         the built-in task · now") && shown.contains("│  \"ops\"               work on other machines over ssh") && shown.contains("\"codex-build\"") && shown.contains("\"codex\""), "{shown}");
        assert!(!shown.contains("\"reviewer\""), "the reviewer is never a task's profile: {shown}");
        let Mode::Choose(chooser) = &fixture.page.mode else { panic!("a pick") };
        let at = chooser.choices.iter().position(|choice| choice.label == "\"codex-build\"").unwrap();
        press(&mut fixture.page, &vec![KeyCode::Down; at]);
        press(&mut fixture.page, &[KeyCode::Enter]);
        assert_eq!(file(&fixture, "config.toml"), "[task]\nprofile = \"codex-build\"\n");
        press(&mut fixture.page, &[KeyCode::Enter]);
        assert!(matches!(&fixture.page.mode, Mode::Choose(chooser) if chooser.index == at), "the pick opens on the profile in effect");
        press(&mut fixture.page, &[KeyCode::Esc]);
        press(&mut fixture.page, &[KeyCode::Enter]);
        press(&mut fixture.page, &vec![KeyCode::Up; at]);
        press(&mut fixture.page, &[KeyCode::Enter]);
        assert!(!file(&fixture, "config.toml").contains("profile"), "none starts the built-in task: the key goes");
    }
}
