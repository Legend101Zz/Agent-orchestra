#![allow(unsafe_code)]

use std::collections::HashSet;
use std::fs;
use std::sync::{Mutex, OnceLock};

use orc_core::inbox::{
    acknowledge_prompt, has_kill, pending_prompts, publish_kill, publish_prompt, read_prompt,
};
use orc_core::registry::{NewRunOptions, new_run};

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(())).lock().unwrap()
}

#[test]
fn prompt_and_kill_protocol_remain_plain_json() {
    let _guard = env_lock();
    let home = std::env::temp_dir().join(format!("orc-rust-inbox-{}", std::process::id()));
    let _ = fs::remove_dir_all(&home);
    // SAFETY: tests serialize environment mutation with env_lock.
    unsafe { std::env::set_var("ORC_HOME", &home) };
    let run = new_run("rpc", &NewRunOptions::compatibility_defaults()).unwrap();
    let prompt = publish_prompt(&run, "keep going").unwrap();
    assert_eq!(read_prompt(&prompt).unwrap().message, "keep going");
    let pending = pending_prompts(&run, &HashSet::new()).unwrap();
    assert_eq!(pending, vec![prompt.clone()]);
    let ack = acknowledge_prompt(&run, &prompt).unwrap();
    assert!(ack.is_file());
    publish_kill(&run).unwrap();
    assert!(has_kill(&run));
    let _ = fs::remove_dir_all(&home);
}
