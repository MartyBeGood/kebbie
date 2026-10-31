use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::permit::Caller;
use crate::sh::Result;

// The playbook's recipes, as the lead names them and as the board shows them.
pub const RECIPES: [(&str, &str); 9] = [
    ("small-fix", "small fix"),
    ("feature-with-ui", "feature with UI"),
    ("task-after-task", "task after task"),
    ("rollout-in-stages", "rollout in stages"),
    ("follow-up", "follow-up"),
    ("research", "research"),
    ("across-repos", "across repos"),
    ("waiting-on-outside", "waiting on outside"),
    ("same-files", "same files"),
];

#[derive(Clone, Debug, PartialEq)]
pub struct Recipe {
    pub name: String,
    pub why: String,
    pub by: String,
}

impl Recipe {
    pub fn shown(&self) -> String {
        format!("recipe: {}", shown_name(&self.name))
    }

    pub fn to_json(&self) -> Value {
        json!({ "name": self.name, "shown": shown_name(&self.name), "why": self.why, "by": self.by })
    }
}

pub fn shown_name(name: &str) -> &str {
    RECIPES.iter().find(|(slug, _)| *slug == name).map_or(name, |(_, shown)| shown)
}

pub fn names() -> String {
    RECIPES.iter().map(|(slug, _)| *slug).collect::<Vec<_>>().join(", ")
}

pub const FORM: &str = "\"<recipe>: <why>\"";

// "small fix: one function and its test" or "small-fix: …", the recipe by either of its names and a why after the colon.
pub fn parse(text: &str) -> Result<(String, String)> {
    let (name, why) = text.split_once(':').ok_or_else(|| format!("name the recipe and why as {FORM}, such as \"small-fix: one function and its test\"; the recipes are {}", names()))?;
    let wanted = name.trim().to_lowercase().split_whitespace().collect::<Vec<_>>().join("-");
    let slug = RECIPES.iter().find(|(slug, shown)| *slug == wanted || shown.to_lowercase().replace(' ', "-") == wanted).map(|(slug, _)| slug.to_string())
        .ok_or_else(|| format!("\"{}\" is no recipe; the recipes are {}", name.trim(), names()))?;
    let why = why.split_whitespace().collect::<Vec<_>>().join(" ");
    if why.is_empty() {
        return Err(format!("say why after the colon, in one line: \"{slug}: <why>\""));
    }
    Ok((slug, why))
}

fn file(state: &Path, address: &str) -> PathBuf {
    state.join("recipes").join(format!("{address}.json"))
}

pub fn load(state: &Path, address: &str) -> Option<Recipe> {
    let value: Value = serde_json::from_str(&fs::read_to_string(file(state, address)).ok()?).ok()?;
    let field = |key: &str| value[key].as_str().unwrap_or_default().to_string();
    Some(Recipe { name: field("name"), why: field("why"), by: field("by") }).filter(|recipe| !recipe.name.is_empty())
}

pub fn save(state: &Path, address: &str, recipe: &Recipe) -> Result<()> {
    crate::state::write(&file(state, address), recipe.to_json().to_string())
}

pub fn forget_in(state: &Path, address: &str) {
    let _ = fs::remove_file(file(state, address));
}

pub fn by(caller: &Caller) -> &'static str {
    match caller {
        Caller::Lead | Caller::LeadSandboxed => "the lead",
        Caller::Task(_) => "the task",
        Caller::User => "you",
        _ => "another session",
    }
}

// The lead names how a task is done before it starts one, so its card and its note carry the choice; you start one by hand without, and a restart keeps the one it has.
pub fn asked_of(caller: &Caller, given: Option<&str>, recorded: bool) -> Result<Option<(String, String)>> {
    match given {
        Some(text) => parse(text).map(Some).map_err(|error| format!("--recipe: {error}")),
        None if !recorded && matches!(caller, Caller::Lead | Caller::LeadSandboxed) => Err(format!(
            "kebbie start from the lead names the task's recipe and why: --recipe {FORM}, such as --recipe \"small-fix: one function and its test\". The skill kebbie:playbook says when each recipe applies: {}.",
            names()
        )),
        None => Ok(None),
    }
}

// Saved for the board and written to the task's note; the returned line is what a vault note's task must write itself.
pub fn record(state: &Path, address: &str, name: &str, why: &str, caller: &Caller, now: u64) -> Result<Option<String>> {
    let recipe = Recipe { name: name.to_string(), why: why.to_string(), by: by(caller).to_string() };
    save(state, address, &recipe)?;
    let line = decision_line(&recipe);
    let note = crate::decisions::write(state, address, &line, now)?;
    Ok(crate::decisions::for_the_task(&note, &line))
}

pub fn decision_line(recipe: &Recipe) -> String {
    format!("{}, because {} ({})", recipe.shown(), recipe.why, recipe.by)
}

const USAGE: &str = "kebbie recipe lists the recipes; kebbie recipe <task> shows the task's; kebbie recipe <task> \"<recipe>: <why>\" sets it";

// kebbie recipe: the recipes, a task's, or a new one for it; a task names only its own.
pub fn command(cwd: &Path, words: &[String]) -> Result<String> {
    command_as(&crate::lead::state_dir()?, &crate::permit::caller(), cwd, words, crate::messages::now(), |address, text| {
        let sessions = crate::peers::sessions()?;
        let task = crate::peers::resolve(address, "kebbie", &sessions)?;
        crate::messages::tell_from_kebbie(&task, text, &sessions)
    })
}

pub(crate) fn command_as(state: &Path, caller: &Caller, cwd: &Path, words: &[String], now: u64, tell: impl FnOnce(&str, &str) -> Result<()>) -> Result<String> {
    let Some(name) = words.first() else {
        let lines: Vec<String> = RECIPES.iter().map(|(slug, shown)| format!("  {slug}  ({shown})")).collect();
        return Ok(format!("The recipes, from the skill kebbie:playbook:\n{}", lines.join("\n")));
    };
    let repo = crate::repo::main_checkout(cwd)?;
    let slot = crate::tasks::find_task(&repo, Some(name), cwd)?;
    let address = crate::peers::address(crate::tasks::task_of(&slot), &crate::repo::name_of(&repo));
    let text = words[1..].join(" ");
    if text.trim().is_empty() {
        return Ok(match load(state, &address) {
            Some(recipe) => format!("{address}: {}", decision_line(&recipe)),
            None => format!("{address} has no recipe: {USAGE}"),
        });
    }
    if let Caller::Task(own) = caller && *own != address {
        return Err(format!("the task {own} names its own recipe, not {address}'s"));
    }
    let (recipe, why) = parse(&text)?;
    let told = record(state, &address, &recipe, &why, caller, now)?;
    let mut said = crate::notice::Notice::new(crate::notice::Tone::Done, "Recipe set").detail("task", &address).detail("recipe", format!("{}, because {why}", shown_name(&recipe)));
    if !matches!(caller, Caller::Task(_)) {
        let text = format!("Your recipe is now {}, because {why}.{}", shown_name(&recipe), told.map(|line| format!(" {line}")).unwrap_or_default());
        if let Err(error) = tell(&address, &text) {
            said = said.detail("its Claude", format!("not told ({error})"));
        }
    } else if let Some(line) = told {
        said = said.next(line);
    }
    Ok(said.to_string())
}

// The line a recipe adds to the task's prompt, so its Claude knows the shape of the work it was given.
pub fn prompt_line(name: &str, why: &str) -> String {
    format!("Recipe: {}, because {why}.", shown_name(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_recipe_is_named_by_its_slug_or_its_shown_name_with_a_why_after_the_colon() {
        assert_eq!(parse("small-fix: one function and its test").unwrap(), ("small-fix".to_string(), "one function and its test".to_string()));
        assert_eq!(parse(" Feature with UI :  a new  settings page ").unwrap(), ("feature-with-ui".to_string(), "a new settings page".to_string()));
        assert_eq!(parse("follow-up: red CI on the pushed commit").unwrap().0, "follow-up");
        let unknown = parse("big rewrite: everything").unwrap_err();
        assert!(unknown.contains("\"big rewrite\" is no recipe") && unknown.contains("small-fix, feature-with-ui"), "{unknown}");
        assert!(parse("small-fix").unwrap_err().contains("name the recipe and why"));
        assert!(parse("small-fix:  ").unwrap_err().contains("say why after the colon"));
        assert_eq!(shown_name("feature-with-ui"), "feature with UI");
        assert_eq!(Recipe { name: "small-fix".to_string(), why: "x".to_string(), by: "the lead".to_string() }.shown(), "recipe: small fix");
    }

    #[test]
    fn the_lead_must_name_a_recipe_to_start_a_task_and_you_need_not() {
        let refused = asked_of(&Caller::Lead, None, false).unwrap_err();
        assert!(refused.contains("--recipe \"<recipe>: <why>\"") && refused.contains("small-fix") && refused.contains("same-files"), "{refused}");
        assert!(asked_of(&Caller::LeadSandboxed, None, false).is_err());
        assert_eq!(asked_of(&Caller::Lead, None, true).unwrap(), None, "a restart keeps the recipe the task has");
        assert_eq!(asked_of(&Caller::User, None, false).unwrap(), None);
        assert_eq!(asked_of(&Caller::Agent, None, false).unwrap(), None);
        assert_eq!(asked_of(&Caller::Lead, Some("research: which queue fits"), false).unwrap(), Some(("research".to_string(), "which queue fits".to_string())));
        assert!(asked_of(&Caller::User, Some("nope: x"), false).unwrap_err().starts_with("--recipe: "), "a recipe you give is checked too");
    }

    #[test]
    fn the_playbook_holds_each_recipe_kebbie_takes_and_no_other() {
        let playbook = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("skills/playbook/SKILL.md")).unwrap();
        let headed: Vec<&str> = playbook.lines().filter_map(|line| line.strip_prefix("### `")?.strip_suffix('`')).collect();
        assert_eq!(headed, RECIPES.map(|(slug, _)| slug), "the skill's recipes are the names kebbie start --recipe takes, in order");
    }

    #[test]
    fn a_saved_recipe_reads_back_and_is_gone_once_forgotten() {
        let state = crate::sh::test_dir("recipe-saved");
        let recipe = Recipe { name: "small-fix".to_string(), why: "one function".to_string(), by: "the lead".to_string() };
        assert_eq!(load(&state, "billing@app"), None);
        save(&state, "billing@app", &recipe).unwrap();
        assert_eq!(load(&state, "billing@app"), Some(recipe));
        forget_in(&state, "billing@app");
        assert_eq!(load(&state, "billing@app"), None);
        let _ = fs::remove_dir_all(state);
    }
}
