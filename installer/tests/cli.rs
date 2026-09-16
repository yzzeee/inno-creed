use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

#[test]
fn cli_handles_help_errors_and_default_prompts_without_gui_or_real_install() {
    let work = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join(".claude-workspace")
        .join(format!("installer-cli-test-{}", std::process::id()));
    std::fs::create_dir_all(&work).unwrap();
    let exe = work.join(if cfg!(windows) {
        "installer-cli.exe"
    } else {
        "installer-cli"
    });
    std::fs::copy(env!("CARGO_BIN_EXE_installer-cli"), &exe).unwrap();

    let help = Command::new(&exe).arg("--help").output().unwrap();
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("installer-cli --uninstall"));
    let missing = Command::new(&exe).stdin(Stdio::null()).output().unwrap();
    assert_eq!(missing.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&missing.stderr).contains("payload"));

    let config = work.join(if cfg!(windows) {
        "roaming/Claude/claude_desktop_config.json"
    } else if cfg!(target_os = "macos") {
        "Library/Application Support/Claude/claude_desktop_config.json"
    } else {
        ".config/Claude/claude_desktop_config.json"
    });
    std::fs::create_dir_all(config.parent().unwrap()).unwrap();
    let original = r#"{"mcpServers":{}}"#;
    std::fs::write(&config, original).unwrap();
    std::fs::create_dir_all(work.join("payload")).unwrap();
    std::fs::copy(
        &exe,
        work.join("payload").join(if cfg!(windows) {
            "inno-creed.exe"
        } else {
            "inno-creed"
        }),
    )
    .unwrap();

    // 첫 Enter로 기본 설정, 두 번째로 기본 설치 위치 — 둘 다 아무것도 입력하지 않아도
    // 기본값이 잡혀야 한다. 마지막은 **명시적인 n**으로 취소한다.
    //
    // 여기서 Enter로 설치까지 진행시키지 않는 것은, 그 다음 단계가 "Claude Desktop이
    // 켜져 있으면 종료 요청"이기 때문이다 — `cargo test`가 개발자의 Claude Desktop을
    // 끄게 둘 수는 없다. Enter가 승낙으로 받아들여지는지는 `cli::tests`의 단위 테스트가 본다.
    let mut child = Command::new(&exe)
        .env("HOME", &work)
        .env("APPDATA", work.join("roaming"))
        .env("LOCALAPPDATA", work.join("local"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"\n\nn\n").unwrap();
    let result = child.wait_with_output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let output = String::from_utf8(result.stdout).unwrap();
    assert!(output.contains("선택한 설정:"));
    assert!(output.contains("대상 폴더:"));
    assert!(output.contains("취소했습니다."));
    assert!(!output.contains("Enter를 누르면 종료합니다"));
    assert_eq!(std::fs::read_to_string(config).unwrap(), original);
    assert!(!work.join("local/inno-creed").exists());
    std::fs::remove_dir_all(work).unwrap();
}
