use system::{CommandLine, NiriFocusSource};

fn main() {
    let source = NiriFocusSource::from_environment();
    if let Err(error) = CommandLine::from_environment().run(&source, std::io::stdout()) {
        eprintln!("system-focus: {error}");
        std::process::exit(1);
    }
}
