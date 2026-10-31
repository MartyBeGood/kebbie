use crate::agent::Agent;

// A title as the board shows it, from what the agent put in its terminal title or kebbie kept of it; None when it names nothing yet. A kept title has no agent.
pub fn clean(agent: Option<Agent>, raw: &str, repo: &str) -> Option<String> {
    if command_line(raw) {
        return None;
    }
    let title = match agent {
        Some(Agent::Codex) => crate::agent::codex::thread_title(raw)?,
        Some(Agent::Claude) => raw.to_string(),
        None => crate::agent::codex::kept_thread_title(raw).unwrap_or_else(|| Some(raw.to_string()))?,
    };
    Some(readable(&title, repo)).filter(|title| !title.is_empty())
}

// The shell titles the terminal with the command it runs, so until the agent names its session the title is its own command line.
fn command_line(title: &str) -> bool {
    let mut words = title.split_whitespace();
    let first = words.next().unwrap_or_default();
    Agent::ALL.iter().any(|agent| first.eq_ignore_ascii_case(agent.key())) && words.next().is_some_and(|word| word.starts_with('-') || word == "resume")
}

// Every task in a repo is about that repo, so a title that opens with its name drops it.
pub fn readable(title: &str, repo: &str) -> String {
    let title = title.trim_start_matches(|character: char| !character.is_alphanumeric()).trim();
    let rest = title.split_once(char::is_whitespace)
        .filter(|(first, _)| first.trim_end_matches([':', ',']).eq_ignore_ascii_case(repo))
        .map_or(title, |(_, rest)| rest.trim_start_matches(['-', '—', '–', ':', '·', ' ']));
    let mut characters = rest.chars();
    characters.next().map(|first| first.to_uppercase().chain(characters).collect()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_title_drops_its_repo_its_marks_and_the_shells_command_line() {
        assert_eq!(readable("Kebbie board task rows", "kebbie"), "Board task rows");
        assert_eq!(readable("✳ kebbie: wrapped CI links", "kebbie"), "Wrapped CI links");
        assert_eq!(readable("Kebbies everywhere", "kebbie"), "Kebbies everywhere");
        assert_eq!(clean(Some(Agent::Claude), "✻ refunds", ""), Some("Refunds".to_string()));
        assert_eq!(clean(Some(Agent::Claude), "claude --resume 1234", ""), None);
        assert_eq!(clean(None, "codex -c 'model_reasoning_effort=\"medium\"' -c", ""), None);
        assert!(!command_line("Codex review of the billing page") && !command_line("Claude Code") && command_line("claude --resume 1234"));
    }

    #[test]
    fn a_codex_title_is_its_thread_title_and_only_codexs_kept_shape_is_read_as_one() {
        assert_eq!(clean(Some(Agent::Codex), "[ ! ] Action Required | Fix the flaky checkout test | wt1.app", "app").as_deref(), Some("Fix the flaky checkout test"));
        assert_eq!(clean(Some(Agent::Codex), "wt1.app", "app"), None, "a Codex titled with its project alone has named no thread yet");
        assert_eq!(clean(None, "Action Required | Fix the flaky checkout test | wt1.app", "").as_deref(), Some("Fix the flaky checkout test"));
        assert_eq!(clean(None, "Action Required | wt1.app", ""), None);
        assert_eq!(clean(None, "Action Required banner on settings page", "").as_deref(), Some("Action Required banner on settings page"), "a Claude title that opens with the words is no Codex title");
        assert_eq!(clean(Some(Agent::Claude), "Action Required | a | b", "").as_deref(), Some("Action Required | a | b"), "a live Claude title is never read as Codex's");
    }
}
