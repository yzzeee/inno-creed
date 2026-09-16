//! installer.exe 옆에 놓인 실제 설치 대상 파일들을 찾는다.
//!
//! `include_bytes!`로 inno-creed 실행 파일을 installer 안에 내장하지 않는다 — "exe 안에
//! exe를 내장했다가 실행 시 디스크에 풀어씀"은 백신·EDR 드로퍼 휴리스틱과 구조적으로
//! 겹친다. 대신 배포 zip 안에 installer와 나란히 두고, 실행 시 그 자리에서 찾는다.
//!
//! 배포 zip 구조:
//! ```text
//! installer.exe (또는 macOS/Linux 실행 파일)
//! installer-cli.exe (또는 macOS/Linux 콘솔 실행 파일)
//! payload/
//!   inno-creed(.exe)
//!   extension/          (Windows만)
//!     manifest.json
//!     background.js
//! ```

use std::path::PathBuf;

pub fn payload_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_default()
        .join("payload")
}

#[cfg(target_os = "windows")]
pub fn inno_creed_binary_name() -> &'static str {
    "inno-creed.exe"
}

#[cfg(not(target_os = "windows"))]
pub fn inno_creed_binary_name() -> &'static str {
    "inno-creed"
}

pub fn payload_binary_path() -> PathBuf {
    payload_dir().join(inno_creed_binary_name())
}

pub fn payload_extension_dir() -> PathBuf {
    payload_dir().join("extension")
}

/// 필수 페이로드가 실제로 옆에 있는지 미리 확인한다. 없으면 zip이 깨졌거나
/// installer만 따로 옮겨서 실행한 경우다 — 이 시점에 바로 알려줘야 한다.
pub fn verify_payload_present() -> Result<(), String> {
    let bin = payload_binary_path();
    if !bin.exists() {
        return Err(format!(
            "설치할 inno-creed 실행 파일을 찾을 수 없습니다: {}\n\
             압축을 푼 폴더 전체(installer와 payload 폴더가 함께)에서 실행해주세요.",
            bin.display()
        ));
    }
    Ok(())
}

pub fn default_install_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        let base = std::env::var_os("LOCALAPPDATA").unwrap_or_default();
        PathBuf::from(base).join("inno-creed")
    }
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var_os("HOME").unwrap_or_default();
        PathBuf::from(home).join("Library/Application Support/inno-creed")
    }
    #[cfg(target_os = "linux")]
    {
        let home = std::env::var_os("HOME").unwrap_or_default();
        PathBuf::from(home).join(".local/share/inno-creed")
    }
}
