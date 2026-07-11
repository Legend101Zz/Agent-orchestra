use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

#[test]
fn rust_only_install_and_uninstall_are_isolated_and_preserve_data() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root =
        std::env::temp_dir().join(format!("orc-install-test-{}-{nonce}", std::process::id()));
    let home = root.join("home");
    let binaries = root.join("bin");
    fs::create_dir_all(&binaries).unwrap();
    fs::create_dir_all(home.join(".pi/agent")).unwrap();
    fs::create_dir_all(home.join(".claude")).unwrap();
    fs::create_dir_all(home.join(".codex")).unwrap();
    fs::write(home.join(".zshrc"), "# existing\n").unwrap();
    fs::write(home.join(".pi/agent/settings.json"), "pi-protected\n").unwrap();
    fs::write(home.join(".claude/settings.json"), "claude-protected\n").unwrap();
    fs::write(home.join(".codex/config.toml"), "codex-protected\n").unwrap();
    for name in ["orc", "orcd", "pi-orchestra"] {
        let path = binaries.join(name);
        fs::write(&path, "#!/bin/sh\nexit 0\n").unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let install = Command::new("bash")
        .arg(repository_root().join("install.sh"))
        .env("HOME", &home)
        .env("ORC_INSTALL_SKIP_BUILD", "1")
        .env("ORC_INSTALL_BIN_DIR", &binaries)
        .output()
        .unwrap();
    assert!(
        install.status.success(),
        "{}",
        String::from_utf8_lossy(&install.stderr)
    );
    for name in ["orc", "orcd", "pi-orchestra"] {
        assert_eq!(
            fs::read_link(home.join(".local/bin").join(name)).unwrap(),
            binaries.join(name)
        );
    }
    assert!(home.join(".orchestra/config.json").is_file());
    fs::write(home.join(".orchestra/keep-me"), "durable\n").unwrap();

    let uninstall = Command::new("bash")
        .arg(repository_root().join("uninstall.sh"))
        .env("HOME", &home)
        .output()
        .unwrap();
    assert!(
        uninstall.status.success(),
        "{}",
        String::from_utf8_lossy(&uninstall.stderr)
    );
    for name in ["orc", "orcd", "pi-orchestra"] {
        assert!(!home.join(".local/bin").join(name).exists());
    }
    assert_eq!(
        fs::read_to_string(home.join(".orchestra/keep-me")).unwrap(),
        "durable\n"
    );
    assert_eq!(
        fs::read_to_string(home.join(".pi/agent/settings.json")).unwrap(),
        "pi-protected\n"
    );
    assert_eq!(
        fs::read_to_string(home.join(".claude/settings.json")).unwrap(),
        "claude-protected\n"
    );
    assert_eq!(
        fs::read_to_string(home.join(".codex/config.toml")).unwrap(),
        "codex-protected\n"
    );
    let _ = fs::remove_dir_all(root);
}
