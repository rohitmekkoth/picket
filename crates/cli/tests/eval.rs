use std::process::Command;

#[test]
fn eval_writes_a_privacy_safe_report_and_requires_mock_flag()
-> Result<(), Box<dyn std::error::Error>> {
    let folder = tempfile::tempdir()?;
    let report = folder.path().join("mock.json");
    let output = Command::new(env!("CARGO_BIN_EXE_picket"))
        .args(["eval", "--mock", "--report"])
        .arg(&report)
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(report)?)?;
    assert_eq!(value["totals"]["cases"], 288);
    assert_eq!(value["gates_passed"], true);
    assert_eq!(value["not_measured"][0], "real-source attribution");
    assert!(value.get("events").is_none());
    let invalid = Command::new(env!("CARGO_BIN_EXE_picket"))
        .arg("eval")
        .output()?;
    assert!(!invalid.status.success());
    Ok(())
}
