// Windows 콘솔 서브시스템: 더블클릭하면 콘솔을 만들고, 셸에서는 종료를 기다린다.
fn main() {
    std::process::exit(installer::cli::main());
}
