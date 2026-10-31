use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[path = "../src/executable.rs"]
mod executable;
#[path = "../src/test_seal.rs"]
mod test_seal;

const SCRIPT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/scripts/install.sh");

struct Home {
    dir: PathBuf,
    home: PathBuf,
}

impl Home {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().canonicalize().unwrap().join(format!("kebbie-install-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let (bin, root, home) = (dir.join("bin"), dir.join("root"), dir.join("home"));
        let tool = |name: &str, body: &str| executable::write(bin.join(name), format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::create_dir_all(&bin).unwrap();
        tool("herdr", "case \"$1\" in --version) echo 'herdr 9.9.9' ;; esac");
        tool("jq", &format!("cat >/dev/null\ncase \"$*\" in *'--arg id kebbie'*) echo 'local {}' ;; esac", root.display()));
        tool("curl", "echo https://github.com/gronare/kebbie/releases/tag/v9.9.9");
        tool("git", "exit 0");
        tool("claude", "exit 1");
        fs::create_dir_all(root.join("target/release")).unwrap();
        executable::write(root.join("target/release/kebbie"), "#!/bin/sh\nexit 0\n").unwrap();
        fs::create_dir_all(&home).unwrap();
        Home { dir, home }
    }

    fn file(&self, path: &str, text: &str) {
        let file = self.home.join(path);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, text).unwrap();
    }

    fn read(&self, path: &str) -> Option<String> {
        fs::read_to_string(self.home.join(path)).ok()
    }

    fn install(&self) -> String {
        let output = Command::new("sh").arg(SCRIPT).env_clear().env("HOME", &self.home).env("PATH", format!("{}:/usr/bin:/bin", self.dir.join("bin").display())).env("NO_COLOR", "1").stdin(std::process::Stdio::null()).output().unwrap();
        let said = format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
        assert!(output.status.success(), "{said}");
        said
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn kelpies_settings_replace_a_kebbie_folder_that_holds_only_logs() {
    let home = Home::new("logs");
    home.file(".config/kelpie/config.toml", "kelpie's\n");
    home.file(".config/kebbie/keeper.log", "log\n");
    home.install();
    assert_eq!(home.read(".config/kebbie/config.toml").as_deref(), Some("kelpie's\n"));
    assert!(!home.home.join(".config/kelpie").exists() && !home.home.join(".config/kebbie.kelpie-backup").exists());
}

#[test]
fn kelpies_folder_is_kept_as_a_backup_beside_a_kebbie_folder_that_holds_more_than_logs() {
    let home = Home::new("both");
    home.file(".config/kelpie/config.toml", "kelpie's\n");
    home.file(".config/kebbie/config.toml", "kebbie's\n");
    home.file(".local/state/kelpie/tasks.json", "kelpie's tasks\n");
    home.file(".local/state/kebbie/notes/billing@app.md", "kebbie's note\n");
    let said = home.install();
    assert_eq!(home.read(".config/kebbie/config.toml").as_deref(), Some("kebbie's\n"), "kebbie's own settings stay");
    assert_eq!(home.read(".config/kebbie.kelpie-backup/config.toml").as_deref(), Some("kelpie's\n"), "{said}");
    assert_eq!(home.read(".local/state/kebbie/notes/billing@app.md").as_deref(), Some("kebbie's note\n"));
    assert_eq!(home.read(".local/state/kebbie.kelpie-backup/tasks.json").as_deref(), Some("kelpie's tasks\n"), "nothing of kelpie's is deleted");
    assert!(!home.home.join(".config/kelpie").exists() && said.contains("kept kelpie's ~/.config/kelpie as ~/.config/kebbie.kelpie-backup"), "{said}");
    assert!(Path::new(&home.home.join(".local/share/kebbie/current")).exists());
}
