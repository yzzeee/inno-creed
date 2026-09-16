//! GUI와 콘솔 인스톨러가 공유하는 설치·제거 로직.

pub mod cli;
pub mod install;
pub mod payload;
#[cfg(windows)]
pub mod registry;
