#[test]
fn doctor_reports_an_inactive_local_runtime() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_picket"))
        .arg("doctor")
        .output()?;
    assert!(output.status.success());
    let info: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(info["monitoring"], false);
    assert_eq!(info["platform"], std::env::consts::OS);
    assert_eq!(info["architecture"], std::env::consts::ARCH);
    assert!(output.stderr.is_empty());
    Ok(())
}
