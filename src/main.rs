use std::env;
use std::ffi::OsString;
use std::io::Write;
use std::process::ExitCode;

const HELP: &str = "\
osavul — статичний аналізатор коду

Використання:
  osavul --help       показати цю довідку
  osavul --version    показати назву й версію

Аналіз ще недоступний.
";

fn main() -> ExitCode {
    // Порівнюємо OsString, щоб аргументи поза UTF-8 теж давали код 2.
    let args: Vec<OsString> = env::args_os().skip(1).collect();
    match args.as_slice() {
        [a] if a == "--help" => {
            print!("{HELP}");
            ExitCode::SUCCESS
        }
        [a] if a == "--version" => {
            println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        _ => {
            let _ = writeln!(
                std::io::stderr(),
                "osavul: непідтримувані аргументи; див. `osavul --help`"
            );
            ExitCode::from(2)
        }
    }
}
