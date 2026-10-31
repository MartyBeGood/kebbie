use std::path::PathBuf;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::agent::Agent;
use crate::board::widgets::{Answer, FAINT, MAUVE, MUTED, RED, SURFACE, Spot, TEAL, TEXT, answer_lines, bold, hint_line, style, wrap};
use crate::profiles::{self, Made, Profile, Role};

pub const STEPS: usize = 6;
const NAME: usize = 0;
const START: usize = 1;
const AGENT: usize = 2;
const MODEL: usize = 3;
const EFFORT: usize = 4;
const ROLE: usize = 5;

#[derive(Clone, Debug, PartialEq)]
pub enum Pick {
    Value(Option<String>),
    Type,
}

pub enum Step {
    Stay,
    Cancel,
    Write { open: bool },
}

pub struct Wizard {
    pub step: usize,
    pub name: String,
    pub starts: Vec<Profile>,
    pub start: usize,
    pub agent: usize,
    pub codex: String,
    pub models: (String, Vec<Answer<Pick>>),
    pub model: usize,
    pub typing: Option<String>,
    pub typed: Option<String>,
    pub efforts: (String, Vec<Answer<Pick>>),
    pub effort: usize,
    pub role: usize,
    pub files: Vec<(String, String)>,
    pub dir: PathBuf,
    pub folder: String,
    pub taken: Vec<String>,
    pub new_role: String,
    pub error: Option<String>,
}

const AGENTS: [Agent; 2] = [Agent::Claude, Agent::Codex];

impl Wizard {
    pub fn new(starts: Vec<Profile>, start: usize, files: Vec<(String, String)>, dir: PathBuf, folder: String, taken: Vec<String>, codex: String) -> Self {
        let mut wizard = Wizard {
            step: NAME, name: String::new(), starts, start, agent: 0, codex, models: (String::new(), Vec::new()), model: 0, typing: None, typed: None,
            efforts: (String::new(), Vec::new()), effort: 0, role: 0, files, dir, folder, taken, new_role: String::new(), error: None,
        };
        wizard.agent = wizard.start_agent();
        wizard
    }

    fn start_agent(&self) -> usize {
        self.from().map_or(0, |profile| AGENTS.iter().position(|agent| *agent == profile.agent).unwrap_or(0))
    }

    pub fn agent(&self) -> Agent {
        AGENTS[self.agent]
    }

    pub fn from(&self) -> Option<&Profile> {
        self.starts.get(self.start)
    }

    pub fn model(&self) -> Option<String> {
        match self.models.1.get(self.model).map(|answer| &answer.then) {
            Some(Pick::Type) => self.typed.clone(),
            Some(Pick::Value(value)) => value.clone(),
            None => None,
        }
    }

    pub fn effort(&self) -> Option<String> {
        match self.efforts.1.get(self.effort).map(|answer| &answer.then) {
            Some(Pick::Value(value)) => value.clone(),
            _ => None,
        }
    }

    pub fn offer_models(&mut self, explain: String, answers: Vec<Answer<Pick>>, selected: usize) {
        self.models = (explain, answers);
        self.model = selected;
    }

    pub fn offer_efforts(&mut self, explain: String, answers: Vec<Answer<Pick>>, selected: usize) {
        self.efforts = (explain, answers);
        self.effort = selected;
    }

    fn name_problem(&self) -> Option<String> {
        let name = self.name.trim();
        if name.is_empty() {
            return Some(String::new());
        }
        if !profiles::valid_name(name) {
            return Some("lower-case letters, digits and dashes, starting with a letter".to_string());
        }
        if profiles::built_in(name).is_some() {
            return Some(format!("{name} is kebbie's own profile; r on its row adds your lines to it"));
        }
        if self.taken.iter().any(|taken| taken == name) {
            return Some(format!("{name} is a profile already, or {name}.toml is in the folder"));
        }
        None
    }

    pub fn roles(&self) -> Vec<Answer<Role>> {
        let new = self.new_role.clone();
        let mut roles = vec![Answer::new("New role", &format!("{new}, empty, yours to write"), Role::New(new.clone()))];
        if let Some(from) = self.from().and_then(|profile| profiles::role_name(&self.dir, profile)) {
            roles.push(Answer::new(&format!("Copy of {from}"), "the same lines, changed for this one alone", Role::Copy { from, to: new }));
        }
        roles.extend(self.files.iter().map(|(file, about)| Answer::new(file, about, Role::File(file.clone()))));
        roles.push(Answer::new("None", "kebbie's role alone", Role::None));
        roles
    }

    pub fn chosen_role(&self) -> Role {
        self.roles().into_iter().nth(self.role).map_or(Role::None, |answer| answer.then)
    }

    pub fn made(&self) -> Option<Made> {
        let role = self.chosen_role();
        Some(profiles::made_from(self.from()?, self.agent(), self.model().as_deref(), self.effort().as_deref(), role.file()))
    }

    fn count(&self) -> usize {
        match self.step {
            START => self.starts.len(),
            AGENT => AGENTS.len(),
            MODEL => self.models.1.len(),
            EFFORT => self.efforts.1.len(),
            ROLE => self.roles().len(),
            _ => 0,
        }
    }

    fn index_mut(&mut self) -> Option<&mut usize> {
        match self.step {
            START => Some(&mut self.start),
            AGENT => Some(&mut self.agent),
            MODEL => Some(&mut self.model),
            EFFORT => Some(&mut self.effort),
            ROLE => Some(&mut self.role),
            _ => None,
        }
    }

    // A click on an answer selects it, leaving a model being typed; true when it was selected already, so the click goes on.
    pub fn choose(&mut self, at: usize) -> bool {
        let typing = self.step == MODEL && self.typing.take().is_some();
        let Some(index) = self.index_mut() else { return false };
        let again = *index == at && !typing;
        *index = at;
        again
    }

    pub fn key(&mut self, key: KeyEvent) -> Step {
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            return Step::Stay;
        }
        if key.code == KeyCode::Esc {
            return Step::Cancel;
        }
        self.error = None;
        if let Some(buffer) = &mut self.typing {
            match key.code {
                KeyCode::Enter if !buffer.trim().is_empty() => {
                    self.typed = Some(buffer.trim().to_string());
                    self.typing = None;
                    self.step += 1;
                }
                KeyCode::Left => self.typing = None,
                KeyCode::Backspace => _ = buffer.pop(),
                KeyCode::Char(character) => buffer.push(character),
                _ => {}
            }
            return Step::Stay;
        }
        match (self.step, key.code) {
            (STEPS, KeyCode::Char('y')) => return Step::Write { open: false },
            (STEPS, KeyCode::Char('g')) => return Step::Write { open: true },
            (NAME, KeyCode::Enter) if self.name_problem().is_none() => {
                self.new_role = profiles::free_role(&self.dir, self.name.trim());
                self.step += 1;
            }
            (NAME, KeyCode::Char(character)) => self.name.push(character),
            (NAME, KeyCode::Backspace) => _ = self.name.pop(),
            (NAME, _) => {}
            (_, KeyCode::Left) => self.step -= 1,
            (STEPS, _) => {}
            (START, KeyCode::Enter) => {
                self.agent = self.start_agent();
                self.step += 1;
            }
            (MODEL, KeyCode::Enter) if self.models.1.get(self.model).is_some_and(|answer| answer.then == Pick::Type) => self.typing = Some(self.typed.clone().unwrap_or_default()),
            (_, KeyCode::Enter) => self.step += 1,
            (_, KeyCode::Up | KeyCode::Char('k')) => {
                if let Some(index) = self.index_mut() {
                    *index = index.saturating_sub(1);
                }
            }
            (_, KeyCode::Down | KeyCode::Char('j')) => {
                let last = self.count().saturating_sub(1);
                if let Some(index) = self.index_mut() {
                    *index = (*index + 1).min(last);
                }
            }
            _ => {}
        }
        Step::Stay
    }

    pub fn paste(&mut self, text: &str) -> bool {
        let text = text.trim_end_matches(['\n', '\r']);
        match (&mut self.typing, self.step) {
            (Some(buffer), _) => buffer.push_str(text),
            (None, NAME) => self.name.push_str(text),
            _ => return false,
        }
        true
    }

    fn input(text: &str) -> Line<'static> {
        Line::from(vec![Span::raw(" "), Span::styled(text.to_string(), style(TEXT)), Span::styled("▏", style(MAUVE))]).style(Style::new().bg(SURFACE))
    }

    pub fn view(&self, width: usize) -> (Vec<Line<'static>>, Vec<(usize, Spot)>) {
        let mut spots = Vec::new();
        let muted = |text: &str| wrap(text, width, 4).into_iter().map(|text| Line::from(Span::styled(text, style(MUTED)))).collect::<Vec<_>>();
        let heading = |text: &str| Line::from(Span::styled(text.to_string(), bold(MAUVE)));
        let mut lines = vec![Line::from(Span::styled(if self.step < STEPS { format!("step {} of {STEPS}", self.step + 1) } else { "check".to_string() }, style(FAINT)))];
        let choose = [("↑ ↓ choose", None), ("⏎ next", Some(KeyCode::Enter.into())), ("← back", Some(KeyCode::Left.into())), ("esc cancel", Some(KeyCode::Esc.into()))];
        let kebbies = self.from().map_or("kebbie's task role", |profile| profiles::kebbie_role(&profile.name).0);
        match self.step {
            NAME => {
                lines.push(heading("Its name?"));
                lines.extend(muted(&format!("The file's name in {}: lower-case letters, digits and dashes.", self.folder)));
                lines.push(Line::default());
                lines.push(Self::input(&self.name));
                lines.push(Line::default());
                match self.name_problem() {
                    Some(problem) if problem.is_empty() => {}
                    Some(problem) => lines.push(Line::from(Span::styled(problem, style(RED)))),
                    None => lines.push(Line::from(Span::styled(format!("{}.toml", self.name.trim()), style(MUTED)))),
                }
                lines.push(Line::default());
                hint_line(&mut lines, &mut spots, &[("⏎ next", Some(KeyCode::Enter.into())), ("esc cancel", Some(KeyCode::Esc.into()))]);
            }
            START => {
                lines.push(heading("Start from which profile?"));
                lines.extend(muted("It takes that one's about, for, skills, permissions and sandbox; never kebbie's role, which every profile reads first."));
                lines.push(Line::default());
                let rows: Vec<Answer<()>> = self.starts.iter().map(|profile| Answer::new(&profile.name, if profile.fits.is_empty() { &profile.about } else { &profile.fits }, ())).collect();
                answer_lines(&mut lines, &mut spots, &rows, self.start, width);
                lines.push(Line::default());
                hint_line(&mut lines, &mut spots, &choose);
            }
            AGENT => {
                lines.push(heading("Which agent?"));
                lines.extend(muted("It runs the task; the model comes next."));
                lines.push(Line::default());
                answer_lines(&mut lines, &mut spots, &[Answer::new("Claude Code", "kebbie's own default", ()), Answer::new("Codex", &self.codex, ())], self.agent, width);
                lines.push(Line::default());
                hint_line(&mut lines, &mut spots, &choose);
            }
            MODEL | EFFORT => {
                let ((explain, answers), selected) = if self.step == MODEL { (&self.models, self.model) } else { (&self.efforts, self.effort) };
                lines.push(heading(if self.step == MODEL { "Which model?" } else { "How much effort?" }));
                lines.extend(muted(explain));
                lines.push(Line::default());
                answer_lines(&mut lines, &mut spots, answers, selected, width);
                if let Some(buffer) = self.typing.as_deref().filter(|_| self.step == MODEL) {
                    lines.push(Line::default());
                    lines.push(Self::input(buffer));
                    lines.push(Line::default());
                    hint_line(&mut lines, &mut spots, &[("type the model's id", None), ("⏎ next", Some(KeyCode::Enter.into())), ("← the list", Some(KeyCode::Left.into())), ("esc cancel", Some(KeyCode::Esc.into()))]);
                } else {
                    lines.push(Line::default());
                    hint_line(&mut lines, &mut spots, &choose);
                }
            }
            ROLE => {
                lines.push(heading("Which role?"));
                lines.extend(muted(&format!("Your lines, read after {kebbies}, which updates with kebbie.")));
                lines.push(Line::default());
                answer_lines(&mut lines, &mut spots, &self.roles(), self.role, width);
                lines.push(Line::default());
                hint_line(&mut lines, &mut spots, &choose);
            }
            _ => {
                lines.push(heading(&format!("Add to {}?", self.folder)));
                lines.push(Line::default());
                lines.push(Line::from(Span::styled(format!("{}.toml", self.name.trim()), style(TEAL))));
                if let Some(made) = self.made() {
                    lines.extend(made.text.lines().map(|line| Line::from(Span::styled(line.to_string(), style(if line.starts_with('[') { TEAL } else { TEXT })))));
                    if !made.dropped.is_empty() {
                        lines.extend(muted(&format!("Left out: {}, which only Claude Code reads.", made.dropped.join(", "))));
                    }
                }
                let role = match self.chosen_role() {
                    Role::New(file) => Some(format!("{file}  new, empty")),
                    Role::Copy { from, to } => Some(format!("{to}  a copy of {from}")),
                    Role::File(file) => Some(format!("{file}  {}", self.files.iter().find(|(name, _)| *name == file).map_or("", |(_, about)| about.as_str()))),
                    Role::None => None,
                };
                if let Some(role) = role {
                    lines.push(Line::default());
                    lines.push(Line::from(Span::styled(role, style(TEXT))));
                }
                if let Some(error) = &self.error {
                    lines.push(Line::default());
                    lines.extend(wrap(error, width, 4).into_iter().map(|text| Line::from(Span::styled(text, style(RED)))));
                }
                lines.push(Line::default());
                let open = if self.chosen_role() == Role::None { "g add and open the file" } else { "g add and open the role" };
                hint_line(&mut lines, &mut spots, &[("y add", Some(KeyCode::Char('y').into())), (open, Some(KeyCode::Char('g').into())), ("← back", Some(KeyCode::Left.into())), ("esc cancel", Some(KeyCode::Esc.into()))]);
            }
        }
        (lines, spots)
    }
}
