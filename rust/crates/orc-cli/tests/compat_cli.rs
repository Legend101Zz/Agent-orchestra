use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../orc-core/tests/fixtures/python-v3")
}

fn copy_tree(source: &Path, target: &Path) {
    fs::create_dir_all(target).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let destination = target.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &destination);
        } else {
            fs::copy(entry.path(), destination).unwrap();
        }
    }
}

fn without_nulls(value: &mut Value) {
    match value {
        Value::Array(values) => values.iter_mut().for_each(without_nulls),
        Value::Object(values) => {
            values.retain(|_, value| !value.is_null());
            values.values_mut().for_each(without_nulls);
        }
        Value::Number(number) => {
            if let Some(float) = number.as_f64()
                && float.fract() == 0.0
                && float >= i64::MIN as f64
                && float <= i64::MAX as f64
            {
                *number = serde_json::Number::from(float as i64);
            }
        }
        _ => {}
    }
}

#[test]
fn rust_cli_matches_normalized_python_json_and_exit_oracle() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("orc-cli-oracle-{}-{nonce}", std::process::id()));
    let home = root.join("orchestra");
    copy_tree(&fixture_root().join("home"), &home);
    let oracle: Value =
        serde_json::from_slice(&fs::read(fixture_root().join("oracle.json")).unwrap()).unwrap();
    for capture in oracle["python"].as_array().unwrap() {
        let args = capture["args"]
            .as_array()
            .unwrap()
            .iter()
            .map(|arg| arg.as_str().unwrap())
            .collect::<Vec<_>>();
        let output = Command::new(env!("CARGO_BIN_EXE_orc"))
            .args(&args)
            .env("ORC_HOME", &home)
            .env("HOME", root.join("empty-home"))
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            capture["exit"].as_i64().map(|code| code as i32),
            "{args:?}"
        );
        let text = String::from_utf8(output.stdout).unwrap();
        let json_text = text.split("\n--- output.log").next().unwrap();
        let normalized = json_text.replace(home.to_str().unwrap(), "<ORC_HOME>");
        let mut actual: Value = serde_json::from_str(&normalized).unwrap();
        let mut expected = capture["stdout"].clone();
        without_nulls(&mut actual);
        without_nulls(&mut expected);
        assert_eq!(actual, expected, "normalized parity failed for {args:?}");
    }
    fs::remove_dir_all(root).unwrap();
}
