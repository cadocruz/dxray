use std::process::Command;

fn tui(arguments: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_dxray-tui"))
        .args(arguments)
        .output()
        .expect("run dxray-tui")
}

#[test]
fn version_and_help_do_not_start_the_terminal_browser() {
    let version = tui(&["--version"]);
    assert!(version.status.success());
    assert_eq!(
        String::from_utf8_lossy(&version.stdout),
        "dxray-tui 0.0.1\n"
    );
    assert!(version.stderr.is_empty());

    let help = tui(&["--help"]);
    assert!(help.status.success());
    let text = String::from_utf8_lossy(&help.stdout);
    assert!(text.contains("USAGE:"));
    assert!(text.contains("--version"));
    assert!(help.stderr.is_empty());
}

#[test]
fn unexpected_arguments_are_usage_errors() {
    let output = tui(&["--unknown"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unexpected argument '--unknown'"));
}
