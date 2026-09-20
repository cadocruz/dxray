use std::process::ExitCode;

fn main() -> ExitCode {
    let mut arguments = std::env::args();
    let program = arguments.next().unwrap_or_else(|| "dxray-tui".to_owned());
    match arguments.next().as_deref() {
        None => match dxray_tui::run::run() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("{program}: {error}");
                ExitCode::FAILURE
            }
        },
        Some("--version" | "-V") if arguments.next().is_none() => {
            println!("dxray-tui {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Some("--help" | "-h") if arguments.next().is_none() => {
            println!(
                "dxray-tui {}\n\nInteractive browser for locally installed Windows games.\n\nUSAGE:\n  dxray-tui\n\nOPTIONS:\n  -h, --help     Print help\n  -V, --version  Print version",
                env!("CARGO_PKG_VERSION")
            );
            ExitCode::SUCCESS
        }
        Some(argument) => {
            eprintln!(
                "{program}: unexpected argument '{argument}'\nTry '{program} --help' for more information."
            );
            ExitCode::from(2)
        }
    }
}
