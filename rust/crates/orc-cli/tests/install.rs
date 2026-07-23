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
    fs::create_dir_all(home.join(".claude/skills")).unwrap();
    fs::create_dir_all(home.join(".codex")).unwrap();
    fs::write(home.join(".zshrc"), "# existing\n").unwrap();
    fs::write(home.join(".pi/agent/settings.json"), "pi-protected\n").unwrap();
    fs::write(home.join(".claude/settings.json"), "claude-protected\n").unwrap();
    fs::write(home.join(".codex/config.toml"), "codex-protected\n").unwrap();
    fs::write(
        home.join(".claude/skills/pi-delegate"),
        "user-owned skill must survive\n",
    )
    .unwrap();
    for name in ["pio", "piod", "pi-orchestra"] {
        let path = binaries.join(name);
        fs::write(&path, "#!/bin/sh\nexit 0\n").unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    // Simulate a prior orc/orcd install so the rename migration (back up the old
    // link, drop a forwarding shim) is exercised end to end.
    let local_bin = home.join(".local/bin");
    fs::create_dir_all(&local_bin).unwrap();
    for name in ["orc", "orcd"] {
        let old_target = binaries.join(format!("{name}-old"));
        fs::write(&old_target, "#!/bin/sh\nexit 0\n").unwrap();
        fs::set_permissions(&old_target, fs::Permissions::from_mode(0o755)).unwrap();
        std::os::unix::fs::symlink(&old_target, local_bin.join(name)).unwrap();
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
    for name in ["pio", "piod", "pi-orchestra"] {
        assert_eq!(
            fs::read_link(local_bin.join(name)).unwrap(),
            binaries.join(name)
        );
    }
    // Old commands are retired to forwarding shims and the prior links backed up.
    for (old, new) in [("orc", "pio"), ("orcd", "piod")] {
        let shim = fs::read_to_string(local_bin.join(old)).unwrap();
        assert!(
            shim.contains("pi-orchestra-rename-shim"),
            "{old} should be a rename shim, got: {shim}"
        );
        assert!(shim.contains(new), "{old} shim should forward to {new}");
        assert_eq!(
            fs::read_link(local_bin.join(format!("{old}.pi-orchestra.bak"))).unwrap(),
            binaries.join(format!("{old}-old")),
            "prior {old} link should be backed up"
        );
    }
    assert!(home.join(".orchestra/config.json").is_file());
    fs::write(home.join(".orchestra/keep-me"), "durable\n").unwrap();

    let reinstall = Command::new("bash")
        .arg(repository_root().join("install.sh"))
        .env("HOME", &home)
        .env("ORC_INSTALL_SKIP_BUILD", "1")
        .env("ORC_INSTALL_BIN_DIR", &binaries)
        .output()
        .unwrap();
    assert!(
        reinstall.status.success(),
        "{}",
        String::from_utf8_lossy(&reinstall.stderr)
    );
    let zshrc = fs::read_to_string(home.join(".zshrc")).unwrap();
    assert_eq!(zshrc.matches("# >>> pi-orchestra >>>").count(), 1);
    let agents = fs::read_to_string(home.join(".codex/AGENTS.md")).unwrap();
    assert_eq!(agents.matches("<!-- pi-orchestra:begin -->").count(), 1);
    assert_eq!(
        fs::read_to_string(home.join(".claude/skills/pi-delegate")).unwrap(),
        "user-owned skill must survive\n"
    );
    // Reinstall is idempotent for the shims too: still a shim, still one backup.
    for old in ["orc", "orcd"] {
        assert!(
            fs::read_to_string(local_bin.join(old))
                .unwrap()
                .contains("pi-orchestra-rename-shim"),
            "{old} stays a shim after reinstall"
        );
        assert!(local_bin.join(format!("{old}.pi-orchestra.bak")).exists());
    }
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
    for name in ["pio", "piod", "pi-orchestra"] {
        assert!(!local_bin.join(name).exists(), "{name} link removed");
    }
    // Uninstall removes our shims and restores the commands we backed up.
    for (old, _) in [("orc", "pio"), ("orcd", "piod")] {
        assert!(
            !local_bin.join(format!("{old}.pi-orchestra.bak")).exists(),
            "{old} backup consumed on uninstall"
        );
        assert_eq!(
            fs::read_link(local_bin.join(old)).unwrap(),
            binaries.join(format!("{old}-old")),
            "{old} restored to its pre-rename link"
        );
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
    assert_eq!(
        fs::read_to_string(home.join(".claude/skills/pi-delegate")).unwrap(),
        "user-owned skill must survive\n"
    );
    let _ = fs::remove_dir_all(root);
}
