use system::SystemCommandLine;

fn main() {
    if let Err(error) = SystemCommandLine::from_env().run(std::io::stdout().lock()) {
        eprintln!("system: {error}");
        std::process::exit(1);
    }
}
