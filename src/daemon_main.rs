use system::SystemDaemonCommand;

fn main() {
    if let Err(error) = SystemDaemonCommand::from_environment().run() {
        eprintln!("system-daemon: {error}");
        std::process::exit(1);
    }
}
