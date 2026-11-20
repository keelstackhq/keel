use assert_cmd::Command;

fn keel() -> Command {
    Command::cargo_bin("keel").unwrap()
}

#[test]
fn help_exits_zero() {
    keel().arg("--help").assert().code(0);
}

#[test]
fn version_exits_zero() {
    keel()
        .arg("--version")
        .assert()
        .code(0)
        .stdout(predicates::str::contains("keel"));
}

#[test]
fn unknown_flag_exits_three_with_json_error() {
    let out = keel().args(["--json", "up", "--nope"]).assert().code(3);
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(v["status"], "error");
    assert_eq!(v["exit_code"], 3);
    assert_eq!(v["command"], "up");
    assert!(v["error"]["message"].as_str().unwrap().contains("--nope"));
}

#[test]
fn unknown_flag_without_json_prints_no_json() {
    keel()
        .args(["up", "--nope"])
        .assert()
        .code(3)
        .stdout(predicates::str::is_empty());
}
