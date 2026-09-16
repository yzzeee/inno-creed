//! 그래픽 초기화 없이 기존 설치·제거 로직을 사용하는 대화형 터미널 진입점.

use crate::{install, payload};
use anyhow::{Context, bail, ensure};
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

pub fn main() -> i32 {
    let own_console = owns_console();
    let mut input = io::stdin().lock();
    let mut output = io::stdout().lock();
    let result = run(&mut input, &mut output);
    let code = match &result {
        Ok(_) => 0,
        Err(error) => {
            eprintln!("오류: {error:#}");
            1
        }
    };
    if own_console {
        let _ = prompt(&mut input, &mut output, "Enter를 누르면 종료합니다: ");
    }
    // 더블클릭 실행의 결과 확인 대기가 끝난 뒤 자기 삭제를 예약한다.
    if let Ok(Some((keep, install_dir))) = result {
        install::schedule_self_delete(&keep, &install_dir);
    }
    code
}

fn run(
    input: &mut impl BufRead,
    output: &mut impl Write,
) -> anyhow::Result<Option<(PathBuf, PathBuf)>> {
    let mut uninstall = false;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--cli" => {}
            "--uninstall" => uninstall = true,
            "--help" | "-h" => {
                writeln!(
                    output,
                    "설치: installer-cli\n제거: installer-cli --uninstall"
                )?;
                return Ok(None);
            }
            _ => bail!("알 수 없는 옵션: {arg}"),
        }
    }
    writeln!(
        output,
        "inno-creed {} 터미널 {}",
        env!("INNO_CREED_RELEASE"),
        if uninstall { "제거" } else { "설치" }
    )?;
    writeln!(output, "취소하려면 Ctrl+C를 누르세요.")?;
    if !uninstall {
        payload::verify_payload_present().map_err(anyhow::Error::msg)?;
    }
    let candidates = config_kit::desktop_config_candidates();
    let config = choose_config(input, output, &candidates, uninstall)?;
    let default_dir = payload::default_install_dir();
    writeln!(output, "기본 설치 폴더: {}", default_dir.display())?;
    let parent = prompt(
        input,
        output,
        "다른 위치라면 상위 폴더의 전체 경로를 입력하세요 (그 안에 inno-creed 폴더 사용, Enter=기본): ",
    )?;
    let install_dir = if parent.is_empty() {
        default_dir
    } else {
        let parent = PathBuf::from(unquote(&parent));
        ensure!(
            parent.is_absolute() && parent.is_dir(),
            "존재하는 상위 폴더의 전체 경로를 입력해주세요."
        );
        parent.canonicalize()?.join("inno-creed")
    };
    let install_dir = if install_dir.exists() {
        install_dir.canonicalize()?
    } else {
        install_dir
    };
    writeln!(output, "대상 폴더: {}", install_dir.display())?;
    let dest_bin = install_dir.join(payload::inno_creed_binary_name());
    if uninstall {
        ensure!(
            dest_bin.is_file(),
            "이 폴더에서 설치된 inno-creed 실행 파일을 찾지 못했습니다."
        );
        if let Some(config) = &config {
            ensure!(
                !config.starts_with(&install_dir),
                "설정 파일이 삭제 대상 폴더 안에 있습니다."
            );
        }
        writeln!(
            output,
            "위 폴더의 모든 파일을 삭제하고 선택한 설정에서 inno-creed 등록을 해제합니다."
        )?;
    } else {
        let source = payload::payload_dir().canonicalize()?;
        ensure!(
            !source.starts_with(&install_dir) && !install_dir.starts_with(&source),
            "배포 payload와 설치 폴더가 겹칩니다. 다른 상위 폴더를 선택해주세요."
        );
        writeln!(
            output,
            "기존 버전: {} / 설치할 버전: {}",
            install::read_version(&dest_bin)
                .as_deref()
                .unwrap_or("없음 또는 확인 불가"),
            install::read_version(&payload::payload_binary_path())
                .as_deref()
                .unwrap_or("확인 불가")
        )?;
    }
    if !confirm(
        input,
        output,
        if uninstall {
            "제거하시겠습니까?"
        } else {
            "설치하시겠습니까?"
        },
    )? {
        writeln!(output, "취소했습니다.")?;
        return Ok(None);
    }
    while config_kit::is_claude_desktop_running() {
        writeln!(output, "Claude Desktop을 트레이에서도 완전히 종료해주세요.")?;
        prompt(
            input,
            output,
            "종료 후 Enter로 다시 확인합니다 (취소: Ctrl+C): ",
        )?;
    }
    if uninstall {
        let keep = install_dir.join("installer-cli.exe");
        install::perform_uninstall(config.as_deref(), &install_dir, &keep)?;
        #[cfg(windows)]
        crate::registry::remove_uninstall_entry();
        writeln!(output, "설치된 파일 제거가 완료됐습니다.")?;
        if config.is_none() {
            writeln!(
                output,
                "설정 파일을 선택하지 않아 등록은 남아 있을 수 있습니다. 직접 확인해주세요."
            )?;
        }
        return Ok(Some((keep, install_dir)));
    }
    let config = config.context("설정 파일이 선택되지 않았습니다.")?;
    // 확인 대기 중 파일이 바뀌었더라도 복사/등록 전에 다시 검사한다.
    validate_config(&config)?;
    let extension = payload::payload_extension_dir();
    let result = install::perform_install(
        &config,
        &install_dir,
        &payload::payload_binary_path(),
        extension.is_dir().then_some(extension.as_path()),
        payload::inno_creed_binary_name(),
    )?;
    #[cfg(windows)]
    {
        let registration = install::copy_installer_self(&install_dir, "installer-cli.exe")
            .map_err(anyhow::Error::from)
            .and_then(|copy| crate::registry::register_uninstall_entry(&install_dir, &copy));
        if let Err(error) = registration {
            writeln!(
                output,
                "설치는 됐지만 Windows 앱 목록 등록은 실패했습니다: {error:#}"
            )?;
        }
    }
    writeln!(output, "설치 완료: {}", result.exe_path.display())?;
    if let Some(backup) = result.backup_path {
        writeln!(output, "기존 설정 백업: {}", backup.display())?;
    }
    if let Some(extension) = result.extension_dir {
        writeln!(
            output,
            "\n확장 프로그램 연결:\n1. Chrome: chrome://extensions 또는 Edge: edge://extensions 를 여세요.\n2. 개발자 모드를 켜고 '압축해제된 확장 프로그램 로드'를 선택하세요.\n3. 다음 폴더를 선택하세요: {}\n4. inno-creed 크레덴셜 브릿지를 켜고 아마란스에 로그인하세요.\nEdge의 확장 사용 해제 경고에서는 '나중에'를 선택하세요.",
            extension.display()
        )?;
    }
    if confirm(
        input,
        output,
        "아마란스 로그인 후 인증 진단(doctor)을 실행하시겠습니까?",
    )? {
        match install::run_doctor(&result.exe_path) {
            Ok(report) => {
                writeln!(output, "{report}")?;
                if !report.contains("✅ 인증 성공") {
                    writeln!(output, "설치·등록은 완료됐지만 인증은 확인되지 않았습니다.")?;
                }
            }
            Err(error) => writeln!(
                output,
                "설치·등록은 완료됐지만 진단 실행에 실패했습니다: {error:#}"
            )?,
        }
    }
    writeln!(output, "Claude Desktop을 다시 실행하면 반영됩니다.")?;
    Ok(None)
}

fn choose_config(
    input: &mut impl BufRead,
    output: &mut impl Write,
    candidates: &[PathBuf],
    optional: bool,
) -> anyhow::Result<Option<PathBuf>> {
    let best = candidates
        .iter()
        .find(|p| config_kit::inspect_config(p) == config_kit::ConfigCheck::LooksLikeClaudeDesktop)
        .or_else(|| {
            candidates.iter().find(|p| {
                config_kit::inspect_config(p) == config_kit::ConfigCheck::ParsedButUnfamiliar
            })
        });
    for path in candidates {
        writeln!(
            output,
            "설정 후보: {} ({:?})",
            path.display(),
            config_kit::inspect_config(path)
        )?;
    }
    if let Some(path) = best {
        writeln!(output, "기본 설정 파일: {}", path.display())?;
    }
    if optional {
        writeln!(
            output,
            "설정을 찾을 수 없다면 '-'를 입력해 등록 해제를 건너뛸 수 있습니다."
        )?;
    }
    loop {
        let answer = prompt(input, output, "설정 JSON의 전체 경로 (Enter=기본): ")?;
        if optional && answer == "-" {
            return Ok(None);
        }
        let path = if answer.is_empty() {
            match best {
                Some(path) => path.clone(),
                None => {
                    writeln!(
                        output,
                        "Claude Desktop을 설치하고 한 번 실행한 뒤 설정 경로를 입력해주세요."
                    )?;
                    continue;
                }
            }
        } else {
            PathBuf::from(unquote(&answer))
        };
        if let Err(error) = validate_config(&path) {
            writeln!(output, "설정 파일을 사용할 수 없습니다: {error:#}")?;
            continue;
        }
        if config_kit::inspect_config(&path) == config_kit::ConfigCheck::ParsedButUnfamiliar
            && !confirm(
                input,
                output,
                "낯선 형식의 JSON입니다. Claude Desktop 설정 파일이 맞습니까?",
            )?
        {
            continue;
        }
        writeln!(output, "선택한 설정: {}", path.display())?;
        return Ok(Some(path.canonicalize()?));
    }
}

fn validate_config(path: &Path) -> anyhow::Result<()> {
    ensure!(
        path.is_absolute() && path.is_file(),
        "존재하는 설정 파일의 전체 경로를 입력해주세요."
    );
    let json = config_kit::read_json(path)?;
    ensure!(json.is_object(), "설정 JSON의 최상위 값은 객체여야 합니다.");
    ensure!(
        json.get("mcpServers").is_none_or(|v| v.is_object()),
        "mcpServers는 객체여야 합니다."
    );
    Ok(())
}

fn unquote(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .unwrap_or(value)
}

fn prompt(
    input: &mut impl BufRead,
    output: &mut impl Write,
    label: &str,
) -> anyhow::Result<String> {
    write!(output, "{label}")?;
    output.flush()?;
    let mut answer = String::new();
    ensure!(
        input.read_line(&mut answer)? != 0,
        "입력이 종료되어 작업을 중단했습니다."
    );
    Ok(answer.trim().to_owned())
}

fn confirm(input: &mut impl BufRead, output: &mut impl Write, label: &str) -> anyhow::Result<bool> {
    loop {
        match prompt(input, output, &format!("{label} [y/N]: "))?
            .to_ascii_lowercase()
            .as_str()
        {
            "y" | "yes" => return Ok(true),
            "" | "n" | "no" => return Ok(false),
            _ => writeln!(output, "y 또는 n을 입력해주세요.")?,
        }
    }
}

#[cfg(not(windows))]
fn owns_console() -> bool {
    false
}

#[cfg(windows)]
fn owns_console() -> bool {
    use std::io::IsTerminal;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetConsoleProcessList(process_list: *mut u32, process_count: u32) -> u32;
    }
    // 셸과 함께 연결되어 있으면 셸이 결과를 보존한다. 자기 혼자 쓰는 콘솔만
    // 닫기 전에 기다린다. 파이프/리다이렉션 입력에는 추가 입력을 요구하지 않는다.
    let mut process_id = 0;
    io::stdin().is_terminal()
        && io::stdout().is_terminal()
        && unsafe { GetConsoleProcessList(&mut process_id, 1) } == 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confirmation_requires_explicit_yes_and_rejects_eof() {
        let mut output = Vec::new();
        assert!(!confirm(&mut &b"\n"[..], &mut output, "설치?").unwrap());
        assert!(!confirm(&mut &b"no\n"[..], &mut output, "설치?").unwrap());
        assert!(confirm(&mut &b"maybe\nYES\n"[..], &mut output, "설치?").unwrap());
        assert!(confirm(&mut &b""[..], &mut output, "설치?").is_err());
    }

    #[test]
    fn missing_config_cannot_be_silently_created() {
        let mut output = Vec::new();
        assert!(choose_config(&mut &b"\n"[..], &mut output, &[], false).is_err());
        assert!(
            choose_config(&mut &b"-\n"[..], &mut output, &[], true)
                .unwrap()
                .is_none()
        );
        assert!(choose_config(&mut &b"-\n"[..], &mut output, &[], false).is_err());
    }
}
