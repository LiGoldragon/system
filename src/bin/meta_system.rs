use system::MetaSystemCommandLine;

fn main() {
    if let Err(error) = MetaSystemCommandLine::from_env().run(std::io::stdout().lock()) {
        eprintln!("meta-system: {error}");
        std::process::exit(1);
    }
}
