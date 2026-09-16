//! 실제 설치 동작 — payload 복사, config-kit으로 등록, 확장 브릿지 연결.
//!
//! 어디서 payload를 찾는지는 이 모듈이 몰라도 된다(그건 `payload` 모듈의 일) — 여기는
//! 이미 찾은 구체적인 경로만 받는다. 그래야 가짜 경로를 넣어 로직만 따로 테스트할 수 있다.

use std::path::{Path, PathBuf};
use std::process::Command;

/// GUI 앱에서 콘솔 프로그램(reg.exe, inno-creed.exe의 doctor 등)을 부르면 순간적으로
/// 검은 콘솔창이 깜빡인다 — Windows에서만 그 창을 안 띄우게 한다.
fn no_console_window(cmd: &mut Command) -> &mut Command {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

pub struct InstallResult {
    pub exe_path: PathBuf,
    pub extension_dir: Option<PathBuf>,
    pub backup_path: Option<PathBuf>,
}

pub fn perform_install(
    config_path: &Path,
    install_dir: &Path,
    src_bin: &Path,
    src_extension_dir: Option<&Path>,
    dest_bin_name: &str,
) -> anyhow::Result<InstallResult> {
    std::fs::create_dir_all(install_dir)?;

    let dest_bin = install_dir.join(dest_bin_name);
    std::fs::copy(src_bin, &dest_bin)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perm = std::fs::metadata(&dest_bin)?.permissions();
        perm.set_mode(0o755);
        std::fs::set_permissions(&dest_bin, perm)?;
    }

    let backup_path = config_kit::backup(config_path)?;
    let mut root = config_kit::read_json(config_path)?;
    config_kit::merge_inno_creed_entry(&mut root, &dest_bin);
    config_kit::write_atomic(config_path, &root)?;

    let mut extension_dir = None;
    if let Some(src_ext) = src_extension_dir.filter(|p| p.exists()) {
        let dest_ext = install_dir.join("extension");
        // 업그레이드 시 이전 버전이 넣어둔 파일이 새 버전엔 없을 수도 있다 — 그냥
        // 겹쳐 쓰면 그런 파일이 영영 남는다. 지우고 다시 채워 항상 새 payload와
        // 정확히 같은 상태로 만든다.
        if dest_ext.exists() {
            std::fs::remove_dir_all(&dest_ext)?;
        }
        copy_dir_all(src_ext, &dest_ext)?;
        // native_host.rs가 이미 구현한 등록 절차를 그대로 재사용 — installer가
        // 레지스트리/매니페스트 작성 로직을 다시 구현하지 않는다. 전 OS 공통이다.
        // 실패해도 설치를 막지 않는다: 설치된 본체가 기동할 때 스스로 다시 등록하고
        // (`native_host::ensure_installed`), 그래도 안 되면 `doctor`가 사유를 보여준다.
        let _ = no_console_window(Command::new(&dest_bin).arg("--install-extension-host")).status();
        extension_dir = Some(dest_ext);
    }

    Ok(InstallResult {
        exe_path: dest_bin,
        extension_dir,
        backup_path,
    })
}

fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let dest_path = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_all(&entry.path(), &dest_path)?;
        } else {
            std::fs::copy(entry.path(), &dest_path)?;
        }
    }
    Ok(())
}

/// 실행 파일의 `--version` 출력에서 버전 문자열만 뽑는다("inno-creed 2.0.0" → "2.0.0").
/// 파일이 없거나 실행이 안 되면 `None` — 그건 "처음 설치"로 다루지 여기서 에러를
/// 내지 않는다. 기존 설치가 있는지, 있다면 몇 버전인지 설치 전에 보여주기 위한 것이다.
pub fn read_version(exe_path: &Path) -> Option<String> {
    if !exe_path.exists() {
        return None;
    }
    let out = no_console_window(Command::new(exe_path).arg("--version")).output().ok()?;
    parse_version_output(&String::from_utf8_lossy(&out.stdout))
}

fn parse_version_output(s: &str) -> Option<String> {
    s.trim().split_whitespace().last().map(str::to_string)
}

/// 설치된 inno-creed의 `doctor`를 실행해 결과 전문을 돌려준다.
pub fn run_doctor(exe_path: &Path) -> anyhow::Result<String> {
    let out = no_console_window(Command::new(exe_path).arg("doctor")).output()?;
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    if !out.stderr.is_empty() {
        text.push_str("\n--- stderr ---\n");
        text.push_str(&String::from_utf8_lossy(&out.stderr));
    }
    Ok(text)
}

/// 인스톨러 자기 자신을 설치 위치에 복사해둔다 — "프로그램 추가/제거"의 제거 항목이
/// 이 사본을 실행하므로, 사용자가 원래 받은 zip을 지워버려도 제거할 수 있다.
pub fn copy_installer_self(install_dir: &Path, installer_exe_name: &str) -> std::io::Result<PathBuf> {
    let src = std::env::current_exe()?;
    let dest = install_dir.join(installer_exe_name);
    if src != dest {
        std::fs::copy(&src, &dest)?;
    }
    Ok(dest)
}

/// 등록 해제 + inno-creed 실행 파일·확장 폴더 삭제. **installer 자기 자신은 지우지
/// 않는다** — Windows는 실행 중인 exe를 스스로 지울 수 없다. 자기 자신 정리는
/// `schedule_self_delete`가 프로세스 종료 후 별도로 처리한다.
/// `config_path`가 `None`이면 등록 해제만 건너뛰고 **파일 삭제는 그대로 한다** —
/// Claude Desktop 설정을 못 찾았다는 이유로 설치된 파일까지 남기면, 사용자는
/// "제거했다"는 화면을 보고도 디스크에 그대로 남은 폴더를 나중에 발견한다.
pub fn perform_uninstall(
    config_path: Option<&Path>,
    install_dir: &Path,
    keep: &Path,
) -> anyhow::Result<()> {
    if let Some(config_path) = config_path
        && config_path.exists()
    {
        let _ = config_kit::backup(config_path);
        let mut root = config_kit::read_json(config_path)?;
        config_kit::remove_inno_creed_entry(&mut root);
        config_kit::write_atomic(config_path, &root)?;
    }
    if install_dir.exists() {
        for entry in std::fs::read_dir(install_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path == keep {
                continue;
            }
            if entry.file_type()?.is_dir() {
                std::fs::remove_dir_all(&path)?;
            } else {
                std::fs::remove_file(&path)?;
            }
        }
    }
    Ok(())
}

/// 프로세스가 끝난 뒤 installer 사본과 (비어 있다면) 설치 폴더까지 지운다.
/// Windows에서 실행 중인 exe는 자기 자신을 못 지우므로, 창을 닫아 프로세스가
/// 끝날 시간을 벌기 위해 점점 길게 대기하며 최대 3번 재시도한다(총 최대 ~9초).
/// 그래도 실패하면 치명적이지 않다 — 등록 해제·데이터 삭제는 이미 끝난 뒤라
/// installer.exe 한 파일만 남는 정도다.
#[cfg(target_os = "windows")]
pub fn schedule_self_delete(installer_exe: &Path, install_dir: &Path) {
    let exe = installer_exe.display();
    let dir = install_dir.display();
    let cmd = format!(
        "ping 127.0.0.1 -n 2 >nul & del /f /q \"{exe}\" || \
         (ping 127.0.0.1 -n 3 >nul & del /f /q \"{exe}\") || \
         (ping 127.0.0.1 -n 5 >nul & del /f /q \"{exe}\") & \
         rmdir \"{dir}\" 2>nul"
    );
    let _ = no_console_window(Command::new("cmd").args(["/C", &cmd])).spawn();
}

#[cfg(not(target_os = "windows"))]
pub fn schedule_self_delete(installer_exe: &Path, install_dir: &Path) {
    // Unix는 실행 중인 파일도 unlink할 수 있으므로 바로 지운다.
    let _ = std::fs::remove_file(installer_exe);
    let _ = std::fs::remove_dir_all(install_dir);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("installer-test-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn parse_version_output_takes_last_token() {
        assert_eq!(parse_version_output("inno-creed 2.0.0\n"), Some("2.0.0".to_string()));
        assert_eq!(parse_version_output("2.0.0"), Some("2.0.0".to_string()));
        assert_eq!(parse_version_output(""), None);
    }

    #[test]
    fn read_version_of_missing_file_is_none() {
        let work = temp_dir("version-missing");
        assert_eq!(read_version(&work.join("does-not-exist")), None);
        std::fs::remove_dir_all(&work).ok();
    }

    #[test]
    fn upgrade_replaces_stale_extension_files() {
        let work = temp_dir("upgrade-ext");
        let src_bin = work.join("fake-inno-creed");
        std::fs::write(&src_bin, b"fake").unwrap();
        let config_path = work.join("claude_desktop_config.json");
        let install_dir = work.join("installed");

        // 1차 설치: manifest.json + old-file.txt(구버전에만 있던 파일)
        let ext_src_v1 = work.join("ext-v1");
        std::fs::create_dir_all(&ext_src_v1).unwrap();
        std::fs::write(ext_src_v1.join("manifest.json"), b"v1").unwrap();
        std::fs::write(ext_src_v1.join("old-file.txt"), b"legacy").unwrap();
        perform_install(&config_path, &install_dir, &src_bin, Some(&ext_src_v1), "inno-creed").unwrap();
        assert!(install_dir.join("extension/old-file.txt").exists());

        // 2차 설치(업그레이드): old-file.txt가 없는 새 payload로 덮어쓴다.
        let ext_src_v2 = work.join("ext-v2");
        std::fs::create_dir_all(&ext_src_v2).unwrap();
        std::fs::write(ext_src_v2.join("manifest.json"), b"v2").unwrap();
        perform_install(&config_path, &install_dir, &src_bin, Some(&ext_src_v2), "inno-creed").unwrap();

        assert!(
            !install_dir.join("extension/old-file.txt").exists(),
            "구버전 잔재 파일이 지워져야 함"
        );
        assert_eq!(
            std::fs::read(install_dir.join("extension/manifest.json")).unwrap(),
            b"v2"
        );

        std::fs::remove_dir_all(&work).ok();
    }

    #[test]
    fn install_merges_config_and_copies_binary_without_extension() {
        let work = temp_dir("basic");
        let src_bin = work.join("fake-inno-creed");
        std::fs::write(&src_bin, b"not a real binary, just bytes").unwrap();

        let config_path = work.join("claude_desktop_config.json");
        std::fs::write(
            &config_path,
            serde_json::to_string(&json!({ "preferences": { "epitaxyPrefs": { "x": 1 } } })).unwrap(),
        )
        .unwrap();

        let install_dir = work.join("installed");
        let result =
            perform_install(&config_path, &install_dir, &src_bin, None, "inno-creed").unwrap();

        assert!(result.exe_path.exists());
        assert!(result.extension_dir.is_none());
        assert!(result.backup_path.is_some());

        let written = config_kit::read_json(&config_path).unwrap();
        assert_eq!(
            written["mcpServers"]["inno-creed"]["command"],
            result.exe_path.to_string_lossy().as_ref()
        );
        // 기존에 있던 무관한 키는 손대지 않아야 한다.
        assert_eq!(written["preferences"]["epitaxyPrefs"]["x"], 1);

        std::fs::remove_dir_all(&work).ok();
    }

    #[test]
    fn install_copies_extension_dir_when_given() {
        let work = temp_dir("ext");
        let src_bin = work.join("fake-inno-creed");
        std::fs::write(&src_bin, b"fake").unwrap();

        let ext_src = work.join("ext-src");
        std::fs::create_dir_all(&ext_src).unwrap();
        std::fs::write(ext_src.join("manifest.json"), b"{}").unwrap();

        let config_path = work.join("claude_desktop_config.json");
        let install_dir = work.join("installed");

        let result = perform_install(
            &config_path,
            &install_dir,
            &src_bin,
            Some(&ext_src),
            "inno-creed",
        )
        .unwrap();

        let ext_dir = result.extension_dir.expect("확장 폴더가 복사됐어야 함");
        assert!(ext_dir.join("manifest.json").exists());

        std::fs::remove_dir_all(&work).ok();
    }

    #[test]
    fn install_on_missing_config_creates_fresh_one() {
        let work = temp_dir("fresh");
        let src_bin = work.join("fake-inno-creed");
        std::fs::write(&src_bin, b"fake").unwrap();

        let config_path = work.join("claude_desktop_config.json"); // 존재하지 않는 파일
        let install_dir = work.join("installed");
        let result =
            perform_install(&config_path, &install_dir, &src_bin, None, "inno-creed").unwrap();

        assert!(result.backup_path.is_none()); // 원본이 없었으니 백업도 없다
        let written = config_kit::read_json(&config_path).unwrap();
        assert!(written["mcpServers"]["inno-creed"]["command"].is_string());

        std::fs::remove_dir_all(&work).ok();
    }

    #[test]
    fn uninstall_removes_registration_but_preserves_other_keys_and_servers() {
        let work = temp_dir("uninstall");
        let src_bin = work.join("fake-inno-creed");
        std::fs::write(&src_bin, b"fake").unwrap();
        let config_path = work.join("claude_desktop_config.json");
        std::fs::write(
            &config_path,
            serde_json::to_string(&json!({
                "preferences": { "epitaxyPrefs": { "x": 1 } },
                "mcpServers": { "other-tool": { "command": "/bin/other" } }
            }))
            .unwrap(),
        )
        .unwrap();
        let install_dir = work.join("installed");
        let result =
            perform_install(&config_path, &install_dir, &src_bin, None, "inno-creed").unwrap();
        assert!(result.exe_path.exists());

        let keep = install_dir.join("installer.exe"); // 이 테스트에선 실제로 존재하지 않음
        perform_uninstall(Some(&config_path), &install_dir, &keep).unwrap();

        assert!(!result.exe_path.exists(), "설치된 inno-creed 실행 파일은 지워져야 함");
        let written = config_kit::read_json(&config_path).unwrap();
        assert!(written["mcpServers"].get("inno-creed").is_none());
        assert_eq!(written["mcpServers"]["other-tool"]["command"], "/bin/other");
        assert_eq!(written["preferences"]["epitaxyPrefs"]["x"], 1);

        std::fs::remove_dir_all(&work).ok();
    }

    #[test]
    fn uninstall_keeps_the_named_file() {
        let work = temp_dir("uninstall-keep");
        let install_dir = work.join("installed");
        std::fs::create_dir_all(&install_dir).unwrap();
        let keep = install_dir.join("installer.exe");
        std::fs::write(&keep, b"self").unwrap();
        std::fs::write(install_dir.join("inno-creed.exe"), b"bin").unwrap();

        let config_path = work.join("claude_desktop_config.json"); // 존재하지 않아도 됨
        perform_uninstall(Some(&config_path), &install_dir, &keep).unwrap();

        assert!(keep.exists(), "keep으로 지정한 파일은 남아있어야 함");
        assert!(!install_dir.join("inno-creed.exe").exists());

        std::fs::remove_dir_all(&work).ok();
    }

    /// Claude Desktop 설정을 못 찾은 환경(config_path=None)에서도 설치된 파일은
    /// 지워져야 한다. 예전에는 이 경우 제거가 통째로 no-op이면서 화면만
    /// "제거 완료"가 떴다.
    #[test]
    fn uninstall_without_config_still_deletes_installed_files() {
        let work = temp_dir("uninstall-noconfig");
        let install_dir = work.join("installed");
        std::fs::create_dir_all(&install_dir).unwrap();
        std::fs::write(install_dir.join("inno-creed.exe"), b"bin").unwrap();
        std::fs::create_dir_all(install_dir.join("extension")).unwrap();
        std::fs::write(install_dir.join("extension/manifest.json"), b"{}").unwrap();

        let keep = install_dir.join("installer.exe"); // 존재하지 않음
        perform_uninstall(None, &install_dir, &keep).unwrap();

        assert!(!install_dir.join("inno-creed.exe").exists());
        assert!(!install_dir.join("extension").exists());

        std::fs::remove_dir_all(&work).ok();
    }
}
