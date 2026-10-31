use std::path::PathBuf;

use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use ratatui::Frame;

use crate::keys::Act;
use crate::next::{self, Next, Part, Row};
use crate::peers;
use crate::tasks;
use super::widgets::{GREEN, MAUVE, MUTED, PEACH, RED, SUBTEXT, SURFACE, TEXT, Spot, bold, centered, cut, hint_line, modal_block, style, text_width};
use super::{App, Column, Hit, Mode, Page, notices};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Window {
    pub selected: usize,
    pub top: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Push {
    pub name: String,
    pub path: PathBuf,
    pub commits: String,
    pub waits: String,
}

const WIDE: usize = 80;

fn icon_color(icon: char) -> Color {
    match icon {
        '?' => PEACH,
        '✗' | '!' => RED,
        '↑' => GREEN,
        _ => MAUVE,
    }
}

fn pad(text: &str, width: usize) -> String {
    let shown = if text_width(text) > width { cut(text, width) } else { text.to_string() };
    let used = text_width(&shown);
    format!("{shown}{}", " ".repeat(width.saturating_sub(used)))
}

fn right(text: &str, width: usize) -> String {
    format!("{}{text}", " ".repeat(width.saturating_sub(text_width(text))))
}

fn row_line(row: &Row, number: usize, widths: (usize, usize, usize), selected: bool, wide: bool, now: u64) -> Line<'static> {
    let (number_width, name_width, inner) = widths;
    let time = if wide { row.when(now) } else { row.age(now) };
    let time_width = if wide { 14 } else { 7 };
    let gap = if wide { "  " } else { " " };
    let fixed = 1 + number_width + gap.len() + 2 + name_width + 2 + 1 + time_width;
    let what_width = inner.saturating_sub(fixed);
    let what = if wide { row.what.as_str() } else { row.short.as_str() };
    let name_style = if selected { bold(TEXT) } else { style(TEXT) };
    let line = Line::from(vec![
        if selected { Span::styled("▸", style(MAUVE)) } else { Span::raw(" ") },
        Span::styled(right(&number.to_string(), number_width), style(if selected { MAUVE } else { MUTED })),
        Span::raw(gap),
        Span::styled(row.icon.to_string(), style(icon_color(row.icon))),
        Span::raw(" "),
        Span::styled(pad(&row.name, name_width), name_style),
        Span::raw("  "),
        Span::styled(pad(what, what_width), style(SUBTEXT)),
        Span::raw(" "),
        Span::styled(right(&time, time_width), style(MUTED)),
    ]);
    if selected { line.style(Style::new().bg(SURFACE)) } else { line }
}

pub(super) fn title(next: &Next) -> String {
    match next.rows.len() {
        0 => "Next".to_string(),
        1 => "Next · 1 needs you".to_string(),
        count => format!("Next · {count} need you"),
    }
}

// The list in the board's popup form: groups as headings over one numbered list, the counts and keys kept at the bottom while the rows scroll.
pub(super) fn draw(frame: &mut Frame, next: &Next, window: &mut Window, now: u64) -> Vec<(Rect, Hit)> {
    let screen = frame.area();
    let width = screen.width.saturating_sub(4).min(100);
    let inner = width.saturating_sub(4) as usize;
    let wide = inner >= WIDE;
    let mut body: Vec<Line<'static>> = Vec::new();
    let mut row_at: Vec<usize> = Vec::new();
    let mut footer: Vec<Line<'static>> = Vec::new();
    let mut spots: Vec<(usize, Spot)> = Vec::new();
    let key = |code: KeyCode| Some(KeyEvent::from(code));
    if next.rows.is_empty() {
        body.push(Line::default());
        body.push(Line::from(vec![Span::styled(" ✓ ", style(GREEN)), Span::styled("Nothing needs you.", bold(TEXT))]));
        body.push(Line::default());
        body.extend(next.on_its_way(now, &crate::recover::local_offset).into_iter().map(|line| Line::from(Span::styled(format!(" {line}"), style(SUBTEXT)))));
        body.push(Line::default());
        hint_line(&mut footer, &mut spots, &[("esc board", key(KeyCode::Esc))]);
    } else {
        window.selected = window.selected.min(next.rows.len() - 1);
        let number_width = next.rows.len().to_string().len();
        let name_width = next.rows.iter().map(|row| text_width(&row.name)).max().unwrap_or(0).min(if wide { 18 } else { 12 });
        if wide {
            body.push(Line::default());
        }
        let mut part: Option<Part> = None;
        for (index, row) in next.rows.iter().enumerate() {
            if part != Some(row.part) {
                if wide && part.is_some() {
                    body.push(Line::default());
                }
                part = Some(row.part);
                body.push(Line::from(Span::styled(format!("{}{}", if wide { " " } else { "" }, row.part.title()), bold(MUTED))));
            }
            row_at.push(body.len());
            body.push(row_line(row, index + 1, (number_width, name_width, inner), index == window.selected, wide, now));
        }
        if wide {
            footer.push(Line::default());
            let counts = next.footer();
            if !counts.is_empty() {
                footer.push(Line::from(Span::styled(counts, style(MUTED))));
            }
        }
    }
    let room = (screen.height.saturating_sub(4) as usize).saturating_sub(footer.len() + 1).max(1);
    let visible = body.len().min(room);
    let max = body.len() - visible;
    if let Some(&line) = row_at.get(window.selected) {
        let header = usize::from(row_at.first() == Some(&line) || next.rows.get(window.selected.wrapping_sub(1)).is_some_and(|before| before.part != next.rows[window.selected].part));
        window.top = window.top.min(line.saturating_sub(header)).max((line + 1).saturating_sub(visible));
    }
    window.top = window.top.min(max);
    if !next.rows.is_empty() {
        let span = next.push_span();
        let push = span.as_ref().map(|span| if wide { format!("P push {span} in order") } else { format!("P push {span}") });
        let mut parts: Vec<(&str, Option<KeyEvent>)> = vec![("⏎ open", key(KeyCode::Enter))];
        if let Some(push) = &push {
            parts.push((push.as_str(), key(KeyCode::Char('P'))));
        }
        if wide {
            parts.push(("↑↓ move", None));
        }
        parts.push((if wide { "esc board" } else { "esc" }, key(KeyCode::Esc)));
        if window.top < max {
            parts.push(("↓ more", None));
        }
        hint_line(&mut footer, &mut spots, &parts);
    }
    let shown: Vec<Line<'static>> = body.iter().skip(window.top).take(visible).cloned().collect();
    let footer_at = shown.len();
    let mut lines = shown;
    lines.extend(footer);
    let area = centered(screen, width, lines.len() as u16 + 2);
    frame.render_widget(Clear, area);
    frame.render_widget(Paragraph::new(lines.clone()).block(modal_block(title(next))), area);
    let mut hits = Vec::new();
    for (index, line) in row_at.iter().enumerate() {
        if (window.top..window.top + visible).contains(line) {
            hits.push((Rect::new(area.x, area.y + 1 + (line - window.top) as u16, area.width, 1), Hit::Choice(index)));
        }
    }
    for (line, spot) in spots {
        if let Spot::Key { at, width, key } = spot {
            hits.push((Rect::new(area.x + 2 + at as u16, area.y + 1 + (footer_at + line) as u16, width as u16, 1), Hit::Key(key.code)));
        }
    }
    hits
}

fn counted(count: usize) -> String {
    const WORDS: [&str; 9] = ["two", "three", "four", "five", "six", "seven", "eight", "nine", "ten"];
    WORDS.get(count.wrapping_sub(2)).map_or(count.to_string(), |word| word.to_string())
}

pub(super) fn push_choices(order: &[Push]) -> Vec<String> {
    match order {
        [one] => vec![format!("Push {}", one.name)],
        [first, ..] => vec![format!("Push all {}", counted(order.len())), format!("Push {} only", first.name)],
        [] => Vec::new(),
    }
}

// Before P sends anything to main, the order it goes in, with only the first as the other answer.
pub(super) fn draw_push_all(frame: &mut Frame, order: &[Push], choice: usize) -> Vec<(Rect, Hit)> {
    let width = 64u16.min(frame.area().width.saturating_sub(4));
    let inner = width.saturating_sub(4) as usize;
    let name_width = order.iter().map(|push| text_width(&push.name)).max().unwrap_or(0).min(inner.saturating_sub(30).max(8));
    let commits_width = order.iter().map(|push| text_width(&push.commits)).max().unwrap_or(0);
    let mut lines = vec![Line::default()];
    for (index, push) in order.iter().enumerate() {
        let waits_width = inner.saturating_sub(4 + 2 + name_width + 2 + commits_width + 2);
        lines.push(Line::from(vec![
            Span::styled(format!("{:>3}  ", index + 1), style(MUTED)), Span::styled(pad(&push.name, name_width), style(TEXT)), Span::raw("  "),
            Span::styled(pad(&push.commits, commits_width), style(SUBTEXT)), Span::raw("  "), Span::styled(pad(&push.waits, waits_width), style(MUTED)),
        ]));
    }
    lines.push(Line::default());
    let about = if order.len() == 1 { "It is rebased onto main and pushed; a conflict or red tests go back to its agent." } else { "Each is rebased onto main and pushed; the first that fails stops the rest and goes back to its agent." };
    lines.extend(super::widgets::wrap(about, inner, 4).into_iter().map(|line| Line::from(Span::styled(line, style(SUBTEXT)))));
    lines.push(Line::default());
    let choices = push_choices(order);
    let choice_at = lines.len();
    let mut spans = Vec::new();
    let mut spots = Vec::new();
    let mut at = 0;
    for (index, label) in choices.iter().enumerate() {
        let text = format!("{} {label}", if index == choice { "▸" } else { " " });
        let shown = if index == choice { bold(MAUVE) } else { style(TEXT) };
        spots.push((at, text_width(&text), index));
        at += text_width(&text) + 4;
        spans.push(Span::styled(text, shown));
        spans.push(Span::raw("    "));
    }
    lines.push(Line::from(spans));
    lines.push(Line::from(Span::styled("← → choose · ⏎ push · esc back", style(MUTED))));
    let area = centered(frame.area(), width, lines.len() as u16 + 2);
    let title = if order.len() == 1 { "Push 1 task".to_string() } else { format!("Push {} tasks in this order", order.len()) };
    frame.render_widget(Clear, area);
    frame.render_widget(Paragraph::new(lines).block(modal_block(title)), area);
    spots.into_iter().map(|(at, width, index)| (Rect::new(area.x + 2 + at as u16, area.y + 1 + choice_at as u16, width as u16, 1), Hit::Choice(index))).collect()
}

impl App {
    pub(super) fn open_next(&mut self) {
        self.mode = Mode::Next(Window::default());
    }

    pub(super) fn handle_next(&mut self, mut window: Window, key: KeyEvent, act: Option<Act>) {
        let next = next::of(&self.tasks);
        let last = next.rows.len().saturating_sub(1);
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => window.selected = window.selected.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => window.selected = (window.selected + 1).min(last),
            KeyCode::PageUp => window.selected = window.selected.saturating_sub(10),
            KeyCode::PageDown => window.selected = (window.selected + 10).min(last),
            KeyCode::Home => window.selected = 0,
            KeyCode::End => window.selected = last,
            KeyCode::Enter => return self.open_row(next.rows.get(window.selected)),
            KeyCode::Char('P') if !next.pushes().is_empty() => return self.ask_push_all(&next, window),
            _ if matches!(act, Some(Act::Back | Act::Close | Act::Next)) => return,
            _ => {}
        }
        self.mode = Mode::Next(window);
    }

    pub(super) fn click_next(&mut self, mut window: Window, index: usize) {
        if window.selected == index {
            return self.open_row(next::of(&self.tasks).rows.get(index));
        }
        window.selected = index;
        self.mode = Mode::Next(window);
    }

    // The row's task selected on the board, then what it needs: its question, its push, its card, else its options.
    fn open_row(&mut self, row: Option<&Row>) {
        self.mode = Mode::Normal;
        let Some(row) = row else { return };
        let Some(index) = self.tasks.iter().position(|item| peers::address(&item.name, &item.place) == row.address) else { return };
        (self.page, self.column, self.work_selected, self.filter, self.in_messages, self.offer_panel.focused, self.comment_panel.focused) = (Page::Work, Column::Work, index, None, false, false, false);
        let item = self.tasks[index].clone();
        match row.part {
            Part::Stopped => match self.question_of(&item) {
                Some(at) => self.open_questions(Some(at)),
                None => self.mode = Mode::Options(0),
            },
            Part::Push => self.act(Act::Push),
            Part::Read if self.card_of(&item).is_some() => self.act(Act::Card),
            Part::Read => self.mode = Mode::Options(0),
        }
    }

    fn ask_push_all(&mut self, next: &Next, window: Window) {
        let order: Vec<Push> = next.pushes().into_iter().filter_map(|row| {
            let item = self.tasks.iter().find(|item| peers::address(&item.name, &item.place) == row.address)?;
            let commits = item.git.as_ref().map_or(0, |git| git.ahead);
            let waits = match row.waits.as_slice() {
                [] => String::new(),
                [one] => format!("{one} waits"),
                many => format!("{} tasks wait", many.len()),
            };
            Some(Push { name: row.name.clone(), path: row.path.clone()?, commits: if commits == 1 { "1 commit".to_string() } else { format!("{commits} commits") }, waits })
        }).collect();
        self.mode = if order.is_empty() { Mode::Next(window) } else { Mode::PushAll { order, choice: 0, back: window } };
    }

    pub(super) fn handle_push_all(&mut self, order: Vec<Push>, choice: usize, back: Window, key: KeyEvent, act: Option<Act>) {
        let count = push_choices(&order).len();
        match key.code {
            KeyCode::Left | KeyCode::Up | KeyCode::Char('h' | 'k') | KeyCode::BackTab => self.mode = Mode::PushAll { order, choice: choice.saturating_sub(1), back },
            KeyCode::Right | KeyCode::Down | KeyCode::Char('l' | 'j') | KeyCode::Tab => self.mode = Mode::PushAll { order, choice: (choice + 1).min(count - 1), back },
            KeyCode::Enter => self.push_all(order, choice),
            _ if matches!(act, Some(Act::Back | Act::Close)) => self.mode = Mode::Next(back),
            _ => self.mode = Mode::PushAll { order, choice, back },
        }
    }

    pub(super) fn push_all(&mut self, order: Vec<Push>, choice: usize) {
        let only_first = choice == 1;
        let order: Vec<(String, PathBuf)> = order.into_iter().take(if only_first { 1 } else { usize::MAX }).map(|push| (push.name, push.path)).collect();
        let label = if order.len() == 1 { format!("Pushing {}…", order[0].0) } else { format!("Pushing {} tasks in order…", order.len()) };
        self.subject = Some(notices::Subject::plain("push"));
        match self.pusher.clone() {
            Some(pusher) => self.in_background(label, move || next::push_in_order(&order, |path| pusher(path))),
            None => self.in_background(label, move || next::push_in_order(&order, |path| tasks::push(path, None, None, false))),
        }
    }
}

pub(super) type Pusher = std::sync::Arc<dyn Fn(&std::path::Path) -> crate::sh::Result<String> + Send + Sync>;

#[cfg(test)]
mod tests {
    use ratatui::crossterm::event::KeyCode;

    use super::*;
    use crate::board::test_support::*;
    use crate::next::tests::{asking, named, now, queued_after, ready, result, working};
    use crate::phase::Phase;
    use crate::status::Item;

    // The mock's day: a question, a red CI only you can settle, three pushes, a result, and what is on its way in.
    pub(crate) fn a_day() -> Vec<Item> {
        let mut red = named(crate::status::tests::task(Some("idle"), Some("ready"), 0), "booking-calendar", "web");
        crate::status::tests::in_phase(&mut red, Phase::CiRed { sha: "abc".to_string() });
        red.phase.as_mut().unwrap().since = now() - 12 * 60;
        red.ci_outside = Some("the deploy window was closed".to_string());
        let mut parked = working("parked-one");
        parked.parked = Some(crate::tasks::Parked { until: None });
        vec![
            asking("billing-vat", "web", 42), red, ready("card-component", "web", 60), ready("fix-grid", "web", 20), ready("copy-typo", "web", 180), result("herdr-app-research", "web", 120),
            queued_after("pricing-section", "web", &["card-component@web"]), queued_after("hero", "web", &["card-component@web"]), queued_after("bump-pins", "web", &["fix-grid@web"]),
            working("one"), working("two"), working("three"), working("four"), parked,
        ]
    }

    fn app_with(items: Vec<Item>) -> App {
        let mut app = test_app(Vec::new());
        app.set_tasks(items);
        app
    }

    fn window(app: &mut App, width: u16, height: u16) -> Vec<String> {
        app.mode = Mode::Next(Window::default());
        render(app, width, height).0
    }

    #[test]
    fn renders_at_100_columns_with_its_groups_counts_and_keys() {
        let mut app = app_with(a_day());
        let rows = window(&mut app, 100, 40);
        let screen = rows.join("\n");
        println!("{screen}");
        assert!(screen.contains("Next · 6 need you"), "{screen}");
        for (heading, row) in [("STOPPED ON YOU", "1  ? billing-vat"), ("PUSH, IN THIS ORDER", "3  ↑ card-component"), ("TO READ", "6  ◆ herdr-app-research")] {
            let (_, heading_y) = find(&rows, heading)[0];
            let (_, row_y) = find(&rows, row).first().copied().unwrap_or_else(|| panic!("{row}: {screen}"));
            assert!(heading_y < row_y, "{screen}");
        }
        assert!(screen.contains("answer its question: Round per invoice or per line?") && screen.contains("waiting 42 min"), "{screen}");
        assert!(screen.contains("2 commits · 2 tasks wait on it") && screen.contains("ready 1 h"), "{screen}");
        assert!(screen.contains("4 working · 3 queued · 1 parked") && screen.contains("⏎ open · P push 3–5 in order · ↑↓ move · esc board"), "{screen}");
    }

    #[test]
    fn renders_at_60_columns_with_the_short_wording() {
        let mut app = app_with(a_day());
        let rows = window(&mut app, 60, 30);
        let screen = rows.join("\n");
        println!("{screen}");
        assert!(rows.iter().all(|row| text_width(row) <= 60), "{screen}");
        assert!(screen.contains("1 ? billing-vat") && screen.contains("answer its question") && !screen.contains("Round per invoice"), "{screen}");
        assert!(screen.contains("CI red, needs your call") && screen.contains("2 wait on it") && screen.contains("result ready"), "{screen}");
        assert!(screen.contains("⏎ open · P push 3–5 · esc"), "{screen}");
    }

    #[test]
    fn an_empty_list_says_nothing_needs_you_and_what_is_on_its_way() {
        let mut parked = working("parked-one");
        parked.parked = Some(crate::tasks::Parked { until: Some(now() + 3600) });
        let mut app = app_with(vec![working("billing-vat"), queued_after("later", "web", &["billing-vat@web"]), parked]);
        let rows = window(&mut app, 100, 30);
        let screen = rows.join("\n");
        println!("{screen}");
        assert!(screen.contains("✓ Nothing needs you.") && screen.contains("1 working · next to come: billing-vat"), "{screen}");
        assert!(screen.contains("1 queued · 1 parked until") && screen.contains("esc board") && !screen.contains("P push"), "{screen}");
    }

    #[test]
    fn a_long_list_scrolls_with_the_selection_and_keeps_its_order() {
        let items: Vec<Item> = (0..30).map(|at| ready(&format!("task-{at:02}"), "web", 300 - at)).collect();
        let mut app = app_with(items);
        let rows = window(&mut app, 100, 20);
        let screen = rows.join("\n");
        assert!(screen.contains("1  ↑ task-00") && !screen.contains("task-29") && screen.contains("↓ more"), "{screen}");
        for _ in 0..29 {
            app.handle_key(key(KeyCode::Down), None);
        }
        let rows = render(&mut app, 100, 20).0;
        let screen = rows.join("\n");
        println!("{screen}");
        assert!(screen.contains("30  ↑ task-29") && !screen.contains("task-00") && !screen.contains("↓ more"), "{screen}");
        let numbers: Vec<usize> = rows.iter().filter_map(|row| row.split("↑ task-").nth(1)).map(|rest| rest[..2].parse().unwrap()).collect();
        assert!(numbers.windows(2).all(|pair| pair[1] == pair[0] + 1), "{numbers:?}");
    }

    #[test]
    fn next_and_the_board_agree() {
        let mut app = app_with(a_day());
        let (rows, _) = render(&mut app, 170, 60);
        let screen = rows.join("\n");
        let groups = [crate::status::Group::NeedsYou, crate::status::Group::ReadyToPush, crate::status::Group::Working, crate::status::Group::Queued];
        let headings: Vec<(usize, crate::status::Group)> = groups.iter().filter_map(|group| find(&rows, group.title()).first().map(|(_, y)| (*y, *group))).collect();
        for row in crate::next::of(&app.tasks).rows {
            let top = headings.iter().map(|(y, _)| *y).min().unwrap();
            let (_, y) = find(&rows, &format!(" {}", row.name)).into_iter().find(|(_, y)| *y > top).unwrap_or_else(|| panic!("{} is not on the board: {screen}", row.name));
            let under = headings.iter().filter(|(heading, _)| *heading < y).max_by_key(|(heading, _)| *heading).map(|(_, group)| *group);
            assert_eq!(under, Some(row.part.board_group()), "{} sits under another group: {screen}", row.name);
        }
    }

    #[test]
    fn kebbie_next_prints_the_same_order_as_the_window() {
        let mut app = app_with(a_day());
        let rows = window(&mut app, 100, 40);
        let next = crate::next::of(&app.tasks);
        let text = next.text(now(), &|_| 0);
        println!("{text}");
        let printed: Vec<&str> = text.lines().collect();
        for (index, row) in next.rows.iter().enumerate() {
            assert!(printed[index].starts_with(&format!("{}  {} {}", index + 1, row.icon, row.name)), "{text}");
            assert!(!find(&rows, &format!("{}  {} {}", index + 1, row.icon, row.name)).is_empty(), "{}", rows.join("\n"));
        }
        assert_eq!(printed.last(), Some(&"4 working · 3 queued · 1 parked"));
    }

    #[test]
    fn p_shows_the_order_and_pushes_it_stopping_at_the_first_failure() {
        let mut app = app_with(a_day());
        let tried = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = tried.clone();
        app.pusher = Some(std::sync::Arc::new(move |path| {
            seen.lock().unwrap().push(path.to_path_buf());
            if path.ends_with("fix-grid") { Err("Not pushed: rules not checked".to_string()) } else { Ok(String::new()) }
        }));
        app.open_next();
        app.handle_key(key(KeyCode::Char('P')), None);
        let rows = render(&mut app, 100, 40).0;
        let screen = rows.join("\n");
        println!("{screen}");
        assert!(screen.contains("Push 3 tasks in this order") && screen.contains("▸ Push all three") && screen.contains("Push card-component only"), "{screen}");
        let order: Vec<usize> = ["1  card-component", "2  fix-grid", "3  copy-typo"].iter().map(|row| find(&rows, row)[0].1).collect();
        assert!(order.windows(2).all(|pair| pair[0] < pair[1]), "{screen}");
        app.handle_key(key(KeyCode::Enter), None);
        let Mode::Working { result, .. } = &app.mode else { panic!("pushing runs in the background") };
        let outcome = result.recv().unwrap();
        assert_eq!(*tried.lock().unwrap(), [PathBuf::from("/work/card-component"), PathBuf::from("/work/fix-grid")]);
        assert!(outcome.unwrap_err().contains("fix-grid did not push, so the rest wait"));
    }

    #[test]
    fn enter_opens_the_question_of_a_task_stopped_on_you_and_the_push_of_a_push() {
        let mut app = app_with(a_day());
        app.questions = app.tasks.iter().filter_map(|item| item.question.clone()).collect();
        app.open_next();
        app.handle_key(key(KeyCode::Enter), None);
        assert!(matches!(app.mode, Mode::Questions(_)), "the question opens in the inbox");
        assert_eq!(app.tasks[app.work_selected].name, "billing-vat", "and its task is selected on the board");
        app.mode = Mode::Next(Window { selected: 1, top: 0 });
        app.handle_key(key(KeyCode::Enter), None);
        assert!(matches!(app.mode, Mode::Options(0)) && app.tasks[app.work_selected].name == "booking-calendar", "a red CI only you can settle opens its options");
        app.mode = Mode::Normal;
        app.handle_key(key(KeyCode::Char('N')), None);
        assert!(matches!(app.mode, Mode::Next(_)), "N opens Next");
        app.handle_key(key(KeyCode::Esc), None);
        assert!(matches!(app.mode, Mode::Normal), "esc goes back to the board");
    }
}
