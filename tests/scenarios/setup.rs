use std::fs;

use crate::lab::{Lab, stdout};

fn row<'a>(said: &'a str, step: &str) -> &'a str {
    said.lines().find(|line| line.split_whitespace().nth(1) == Some(step)).unwrap_or_else(|| panic!("no {step} row: {said}"))
}

fn doctor_line(doctor: &str, name: &str) -> String {
    doctor.lines().find(|line| line.get(6..).is_some_and(|rest| rest.starts_with(name))).unwrap_or_else(|| panic!("no {name} line: {doctor}")).to_string()
}

#[test]
fn setup_yes_on_a_new_home_installs_the_claude_plugin_and_herdrs_keys_and_ends_on_the_doctors_count() {
    let lab = Lab::new("setup");
    let said = lab.ok(&["setup", "--yes"]);
    assert!(row(&said, "plugin").starts_with("done") && row(&said, "plugin").contains("installed kebbie@kebbie in Claude Code"), "{said}");
    assert!(row(&said, "keys").starts_with("done") && row(&said, "keys").contains("added prefix+space"), "{said}");
    assert!(row(&said, "link").starts_with("skipped"), "without the current link there is nothing to link: {said}");
    assert!(said.lines().any(|line| line.starts_with("Setup done · doctor: ")), "{said}");
    let plugin = fs::read_to_string(lab.root.join("claude-plugin.log")).unwrap();
    assert_eq!(plugin.lines().map(|line| line.split_whitespace().take(3).collect::<Vec<_>>().join(" ")).collect::<Vec<_>>(), ["plugin marketplace add", "plugin install kebbie@kebbie"], "{plugin}");
    let keys = fs::read_to_string(lab.home.join(".config/herdr/config.toml")).unwrap();
    assert!(keys.contains("kebbie"), "{keys}");
    let again = lab.ok(&["setup", "--yes"]);
    assert!(!row(&again, "keys").starts_with("done"), "a second run adds no keys: {again}");
    assert_eq!(fs::read_to_string(lab.home.join(".config/herdr/config.toml")).unwrap(), keys);
}

#[test]
fn setup_codex_yes_installs_the_plugin_and_only_offers_the_rules_file_while_codexs_config_stays_as_it_was() {
    let lab = Lab::new("setup-codex");
    let codex = lab.home.join(".codex");
    fs::create_dir_all(&codex).unwrap();
    let config = "# mine\nmodel = \"gpt-5.1-codex\"\n";
    fs::write(codex.join("config.toml"), config).unwrap();
    let doctor = stdout(&lab.run(&["doctor"]));
    assert!(doctor_line(&doctor, "codex plugin").starts_with("warn"), "{doctor}");
    let said = lab.ok(&["setup", "codex", "--yes"]);
    assert!(said.starts_with("kebbie's Codex plugin installed at "), "{said}");
    assert!(said.contains("kebbie setup codex in a terminal also offers, each on your choice:") && said.contains("approvals  ~/.codex/rules/kebbie.rules"), "{said}");
    assert!(!codex.join("rules").exists(), "the rules file is written only when the guide applies it");
    assert_eq!(fs::read_to_string(codex.join("config.toml")).unwrap(), config, "no server table or other key is added to Codex's own config");
    let doctor = stdout(&lab.run(&["doctor"]));
    assert!(doctor_line(&doctor, "codex plugin").starts_with("ok"), "{doctor}");
    assert!(doctor_line(&doctor, "codex approvals").starts_with("warn") && doctor.contains("kebbie.rules is missing"), "{doctor}");
    assert!(lab.ok(&["setup", "codex", "--yes"]).starts_with("kebbie's Codex plugin is up to date at "));
}

#[test]
fn setup_yes_sets_up_only_the_agents_chosen_and_each_agents_own_setup_writes_its_own_part() {
    let lab = Lab::new("setup-agents");
    let claude_only = lab.ok(&["setup", "--yes"]);
    assert!(!claude_only.lines().any(|line| line.split_whitespace().nth(1) == Some("codex")), "Codex was never chosen, so setup leaves it: {claude_only}");
    assert!(!lab.home.join(".codex").exists());
    let claude = lab.ok(&["setup", "claude", "--yes"]);
    assert!(claude.lines().next().unwrap().contains("plugin") && !claude.contains("Codex"), "{claude}");
    lab.ok(&["setup", "codex", "--yes"]);
    let both = lab.ok(&["setup", "--yes"]);
    assert!(row(&both, "codex").starts_with("already") && row(&both, "codex").contains("kebbie's Codex plugin is up to date"), "{both}");
}

#[test]
fn the_doctor_in_the_suite_reads_only_the_lab_and_the_systems_own_tools() {
    let lab = Lab::new("setup-doctor");
    let doctor = stdout(&lab.run(&["doctor"]));
    let root = lab.root.display().to_string();
    let outside = doctor.replace(&root, "<lab>");
    let facts: Vec<&str> = outside.lines().filter(|line| !line.trim_start().starts_with("fix:")).collect();
    for host in [" /Users/", " /home/", " /opt/homebrew", ".cargo/bin", " /usr/local/bin"] {
        let leaked: Vec<&&str> = facts.iter().filter(|line| line.contains(host)).collect();
        assert!(leaked.is_empty(), "{host} reached the doctor: {leaked:?}");
    }
    for tool in ["herdr", "claude", "codex"] {
        assert!(doctor_line(&doctor, tool).contains(&root), "{tool} is the lab's: {doctor}");
    }
    assert!(!doctor.contains("push block"), "no checkout around the lab, so no push block line: {doctor}");
}
