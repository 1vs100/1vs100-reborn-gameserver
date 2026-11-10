use std::process::Command;

#[test]
fn startup_cli_rejects_malformed_bind_with_nonzero_exit() {
    let output = Command::new(env!("CARGO_BIN_EXE_onevs100-gameserver"))
        .env("GAMESERVER_BIND", "not an ip")
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("GAMESERVER_BIND"), "stderr was: {stderr}");
}

#[test]
fn startup_cli_requires_a_bind_address() {
    let output = Command::new(env!("CARGO_BIN_EXE_onevs100-gameserver"))
        .env_remove("GAMESERVER_BIND")
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("GAMESERVER_BIND is required"),
        "stderr was: {stderr}"
    );
}

#[test]
fn startup_cli_requires_a_protocol() {
    let output = Command::new(env!("CARGO_BIN_EXE_onevs100-gameserver"))
        .env("GAMESERVER_BIND", "127.0.0.1")
        .env_remove("GAMESERVER_PROTOCOL")
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("GAMESERVER_PROTOCOL is required"),
        "stderr was: {stderr}"
    );
}
