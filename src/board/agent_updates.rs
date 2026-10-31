use std::path::Path;
use std::sync::mpsc::{self, Receiver};
use std::thread;

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::text::{Line, Span};

use crate::agent::codex_setup::{Installer, short_version};
use crate::agent_check::Found;
use crate::config::AgentUpdates;
use crate::sh::Result;
use super::widgets::{Answer, GREEN, MUTED, MAUVE, RED, TEXT, answer_lines, bold, modal, style};
use super::{App, Mode};

const WIDTH: u16 = 72;

pub(super) type Install = std::sync::Arc<dyn Fn(Installer) -> Result<String> + Send + Sync>;

pub(super) struct Offer {
    found: Found,
    installer: Installer,
    choice: usize,
    auto: bool,
    run: Option<Receiver<Result<String>>>,
    pub(super) outcome: Option<Result<String>>,
}

impl Offer {
    fn latest(&self) -> String {
        self.found.latest.clone().unwrap_or_default()
    }

    fn installing(&self) -> bool {
        self.run.is_some() && self.outcome.is_none()
    }
}

// Codex's own installer, run as Codex's update screen runs it; the check's record then names the version now installed.
pub(super) fn default_install() -> Install {
    std::sync::Arc::new(|installer: Installer| {
        let (program, args) = installer.command();
        let said = crate::sh::run(program, args, None)?;
        if let Ok(state) = crate::lead::state_dir() {
            crate::agent_check::installed_now(&state);
        }
        Ok(said)
    })
}

fn muted(text: impl Into<String>) -> Line<'static> {
    Line::from(Span::styled(text.into(), style(MUTED)))
}

pub(super) fn lines(offer: &Offer, width: usize) -> Vec<Line<'static>> {
    let latest = short_version(&offer.latest());
    let installed = short_version(offer.found.installed.as_deref().unwrap_or_default());
    let mut lines = Vec::new();
    match &offer.outcome {
        Some(Ok(_)) => {
            lines.push(Line::from(Span::styled(format!("✓ Codex {latest} is installed"), bold(GREEN))));
            lines.push(Line::default());
            lines.push(muted("Codex tasks restart on the new version once they are idle."));
            lines.push(Line::default());
            lines.push(muted("any key closes"));
        }
        Some(Err(error)) => {
            lines.push(Line::from(Span::styled("✗ Codex's installer stopped", bold(RED))));
            lines.push(Line::from(Span::styled(error.lines().next().unwrap_or_default().to_string(), style(TEXT))));
            lines.push(Line::default());
            lines.push(muted(format!("Codex stays on {installed}; run {} yourself.", offer.installer.command_line())));
            lines.push(Line::default());
            lines.push(muted("any key closes"));
        }
        None if offer.installing() => {
            lines.push(Line::from(Span::styled(format!("Updating Codex {installed} → {latest} with its own installer…"), style(MAUVE))));
            if offer.auto {
                lines.push(muted("update.agents is \"auto\", so U updates Codex too."));
            }
            lines.push(Line::default());
            lines.push(muted("Codex tasks restart on the new version once they are idle."));
        }
        None => {
            lines.push(Line::from(Span::styled("Which updates?", bold(MAUVE))));
            lines.push(muted("kebbie is updated. These agents have a newer version:"));
            lines.push(Line::default());
            let answers = [Answer::new(&format!("Codex {installed} → {latest}"), &format!("{} · its own installer", offer.installer.shown()), ()), Answer::new("Skip this time", "asked again on the next U", ())];
            answer_lines(&mut lines, &mut Vec::new(), &answers, offer.choice, width);
            lines.push(Line::default());
            lines.push(muted("Codex tasks restart on the new version once they are idle."));
            lines.push(muted("Claude Code updates itself, so it is not listed."));
            lines.push(Line::default());
            lines.push(muted("↑ ↓ choose · ⏎ install · esc close"));
        }
    }
    lines
}

pub(super) fn draw(frame: &mut Frame, app: &App) {
    if let Some(offer) = &app.agent_offer {
        modal(frame, "agent updates".to_string(), lines(offer, WIDTH as usize - 4), WIDTH);
    }
}

impl App {
    // After kebbie's own update: an agent that does not update itself and has a newer version is offered, installed at once under "auto", and left to the notice under "notify".
    pub(super) fn offer_agent_updates(&mut self, state: &Path) {
        let mode = (self.agent_mode)();
        if mode == AgentUpdates::Notify {
            return;
        }
        let Some(found) = crate::agent_check::read(state).filter(|found| found.newer().is_some()) else { return };
        let Some(installer) = found.installer else { return };
        let mut offer = Offer { found, installer, choice: 0, auto: mode == AgentUpdates::Auto, run: None, outcome: None };
        if offer.auto {
            self.start_install(&mut offer);
        }
        self.agent_offer = Some(offer);
        self.mode = Mode::AgentUpdates;
    }

    fn start_install(&self, offer: &mut Offer) {
        let install = self.installer.clone();
        let installer = offer.installer;
        let (sender, result) = mpsc::channel();
        thread::spawn(move || {
            let _ = sender.send(install(installer));
        });
        offer.run = Some(result);
    }

    pub(super) fn poll_agent_install(&mut self) {
        let Some(offer) = self.agent_offer.as_mut().filter(|offer| offer.installing()) else { return };
        if let Some(outcome) = offer.run.as_ref().and_then(|run| run.try_recv().ok()) {
            offer.outcome = Some(outcome);
            let _ = self.refresh.send(());
        }
    }

    pub(super) fn handle_agent_updates(&mut self, key: KeyEvent) {
        let Some(mut offer) = self.agent_offer.take() else { return };
        self.mode = Mode::AgentUpdates;
        if offer.installing() {
            self.agent_offer = Some(offer);
            return;
        }
        if offer.outcome.is_some() {
            self.mode = Mode::Normal;
            return;
        }
        match key.code {
            KeyCode::Up | KeyCode::Down => offer.choice = 1 - offer.choice,
            KeyCode::Enter if offer.choice == 0 => self.start_install(&mut offer),
            KeyCode::Enter | KeyCode::Esc | KeyCode::Char('q') => {
                self.mode = Mode::Normal;
                return;
            }
            _ => {}
        }
        self.agent_offer = Some(offer);
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;
    use crate::board::test_support::*;

    fn checked(dir: &Path) {
        let found = Found { checked_at: 1, installed: Some("0.163.0".to_string()), latest: Some("0.164.0".to_string()), installer: Some(Installer::Standalone), ..Found::default() };
        crate::agent_check::write_for_test(dir, &found);
    }

    fn waits(app: &mut App) {
        let started = Instant::now();
        while app.agent_offer.as_ref().is_some_and(Offer::installing) {
            assert!(started.elapsed() < Duration::from_secs(10), "the install never ended");
            thread::sleep(Duration::from_millis(10));
            app.poll_agent_install();
        }
    }

    fn installs(app: &mut App) -> std::sync::Arc<std::sync::Mutex<Vec<Installer>>> {
        let ran = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = ran.clone();
        app.installer = std::sync::Arc::new(move |installer| {
            seen.lock().unwrap().push(installer);
            Ok("installed".to_string())
        });
        ran
    }

    #[test]
    fn after_u_a_newer_codex_is_offered_and_installs_only_on_enter() {
        let dir = crate::sh::test_dir("agent-updates");
        checked(&dir);
        let mut app = test_app(vec![crate::status::tests::task(Some("idle"), None, 0)]);
        app.agent_mode = std::sync::Arc::new(|| AgentUpdates::Ask);
        let ran = installs(&mut app);
        app.offer_agent_updates(&dir);
        assert!(matches!(app.mode, Mode::AgentUpdates));
        let shown = render(&mut app, 100, 30).0.join("\n");
        println!("--- agent updates popup ---\n{shown}");
        for line in ["╭ agent updates ", "Which updates?", "kebbie is updated. These agents have a newer version:", "▌Codex 0.163 → 0.164", "standalone install · its own installer", "Skip this time", "asked again on the next U", "Codex tasks restart on the new version once they are idle.", "Claude Code updates itself, so it is not listed.", "↑ ↓ choose · ⏎ install · esc close"] {
            assert!(shown.contains(line), "{line:?} missing from:\n{shown}");
        }
        assert!(!app.may_restart(), "the board waits for the window before it takes the new kebbie");
        press(&mut app, key(KeyCode::Down));
        assert!(render(&mut app, 100, 30).0.join("\n").contains("▌Skip this time"));
        press(&mut app, key(KeyCode::Up));
        assert!(ran.lock().unwrap().is_empty(), "nothing installs before ⏎");
        press(&mut app, key(KeyCode::Enter));
        waits(&mut app);
        assert_eq!(*ran.lock().unwrap(), [Installer::Standalone]);
        assert!(render(&mut app, 100, 30).0.join("\n").contains("✓ Codex 0.164 is installed"));
        press(&mut app, key(KeyCode::Char('x')));
        assert!(matches!(app.mode, Mode::Normal) && app.agent_offer.is_none() && app.may_restart());
        let mut skip = test_app(Vec::new());
        skip.agent_mode = std::sync::Arc::new(|| AgentUpdates::Ask);
        let ran = installs(&mut skip);
        skip.offer_agent_updates(&dir);
        press(&mut skip, key(KeyCode::Down));
        press(&mut skip, key(KeyCode::Enter));
        assert!(matches!(skip.mode, Mode::Normal) && ran.lock().unwrap().is_empty(), "Skip this time installs nothing");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn auto_installs_after_u_and_says_so_and_notify_shows_no_window() {
        let dir = crate::sh::test_dir("agent-updates-auto");
        checked(&dir);
        let mut app = test_app(Vec::new());
        app.agent_mode = std::sync::Arc::new(|| AgentUpdates::Auto);
        let (release, gate) = mpsc::channel::<()>();
        let gate = std::sync::Mutex::new(gate);
        app.installer = std::sync::Arc::new(move |_| gate.lock().unwrap().recv().map(|_| "installed".to_string()).map_err(|error| error.to_string()));
        app.offer_agent_updates(&dir);
        let shown = render(&mut app, 100, 30).0.join("\n");
        assert!(shown.contains("Updating Codex 0.163 → 0.164 with its own installer…") && shown.contains("update.agents is \"auto\", so U updates Codex too."), "{shown}");
        press(&mut app, key(KeyCode::Esc));
        assert!(matches!(app.mode, Mode::AgentUpdates), "the window stays while the installer runs");
        release.send(()).unwrap();
        waits(&mut app);
        assert!(render(&mut app, 100, 30).0.join("\n").contains("✓ Codex 0.164 is installed"));
        let mut quiet = test_app(Vec::new());
        quiet.agent_mode = std::sync::Arc::new(|| AgentUpdates::Notify);
        quiet.offer_agent_updates(&dir);
        assert!(matches!(quiet.mode, Mode::Normal) && quiet.agent_offer.is_none(), "notify is the notice and doctor's line alone");
        let up_to_date = crate::sh::test_dir("agent-updates-none");
        let mut none = test_app(Vec::new());
        none.agent_mode = std::sync::Arc::new(|| AgentUpdates::Ask);
        none.offer_agent_updates(&up_to_date);
        assert!(matches!(none.mode, Mode::Normal), "no check, nothing to offer");
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(up_to_date);
    }
}
