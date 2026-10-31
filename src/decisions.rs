use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::sh::Result;

// A task's history lives once, in its note: the vault's when it has one, else kebbie's own in its state, both with a Decisions section.
#[derive(Debug, PartialEq)]
pub enum Note {
    Vault(String),
    Local(PathBuf),
}

pub fn local_file(state: &Path, address: &str) -> PathBuf {
    state.join("notes").join(format!("{address}.md"))
}

pub fn of(state: &Path, address: &str) -> Note {
    match crate::task_note::linked(state, address) {
        Some(note) => Note::Vault(note),
        None => Note::Local(local_file(state, address)),
    }
}

// kebbie writes its own note; a vault note is the task's Claude's to write, so the caller hands it the line.
pub fn write(state: &Path, address: &str, line: &str, now: u64) -> Result<Note> {
    let note = of(state, address);
    if let Note::Local(file) = &note {
        append(file, address, &format!("- {}: {line}", crate::issues::today(now)))?;
    }
    Ok(note)
}

fn append(file: &Path, address: &str, line: &str) -> Result<()> {
    let fresh = !file.exists();
    if fresh {
        crate::state::write(file, format!("# {address}\n\n## Decisions\n\n"))?;
    }
    let mut opened = fs::OpenOptions::new().append(true).open(file).map_err(|error| format!("{}: {error}", file.display()))?;
    writeln!(opened, "{line}").map_err(|error| format!("{}: {error}", file.display()))
}

// What the task's Claude is told when the decision belongs in its vault note.
pub fn for_the_task(note: &Note, line: &str) -> Option<String> {
    match note {
        Note::Vault(note) => Some(format!("Write this in the decisions section of your note {note}, with the vault tools: {line}")),
        Note::Local(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_a_vault_note_kebbie_keeps_the_tasks_decisions_in_its_own_note() {
        let state = crate::sh::test_dir("decisions-local");
        let note = write(&state, "billing@app", "recipe: small fix, because one function and its test (the lead)", 1_792_670_400).unwrap();
        assert_eq!(note, Note::Local(state.join("notes/billing@app.md")));
        write(&state, "billing@app", "review suggested: changes how VAT is rounded", 1_792_670_400).unwrap();
        assert_eq!(fs::read_to_string(state.join("notes/billing@app.md")).unwrap(), "# billing@app\n\n## Decisions\n\n- 2026-10-22: recipe: small fix, because one function and its test (the lead)\n- 2026-10-22: review suggested: changes how VAT is rounded\n");
        assert_eq!(for_the_task(&note, "x"), None, "kebbie wrote it itself");
        let _ = fs::remove_dir_all(state);
    }

    #[test]
    fn with_a_vault_note_the_line_goes_to_the_task_and_kebbie_writes_no_note_of_its_own() {
        let state = crate::sh::test_dir("decisions-vault");
        fs::create_dir_all(state.join("started")).unwrap();
        fs::write(state.join("started/billing@app"), "Projects/app-billing.md\n").unwrap();
        let note = write(&state, "billing@app", "review suggested: changes how VAT is rounded", 1_792_670_400).unwrap();
        assert_eq!(note, Note::Vault("Projects/app-billing.md".to_string()));
        assert!(!state.join("notes").exists());
        assert_eq!(for_the_task(&note, "review suggested: changes how VAT is rounded").unwrap(), "Write this in the decisions section of your note Projects/app-billing.md, with the vault tools: review suggested: changes how VAT is rounded");
        let _ = fs::remove_dir_all(state);
    }
}
