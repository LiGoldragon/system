use nota_config::ConfigurationSource;
use signal_system::SystemDaemonConfiguration;
use system::{SystemCommandLine, daemon::SystemDaemon};

fn main() {
    // The supervised production launch passes a typed
    // `SystemDaemonConfiguration` as argv[1]. The same binary also
    // serves the legacy positional `<socket>` CLI surface; pick the
    // typed path when argv looks like a configuration source.
    if first_argument_is_typed_configuration_source() {
        if let Err(error) = run_from_configuration() {
            eprintln!("system-daemon: {error}");
            std::process::exit(1);
        }
        return;
    }
    if let Err(error) = SystemCommandLine::from_environment().run() {
        eprintln!("system-daemon: {error}");
        std::process::exit(1);
    }
}

fn run_from_configuration() -> Result<(), system::error::Error> {
    let configuration: SystemDaemonConfiguration = ConfigurationSource::from_argv()?.decode()?;
    SystemDaemon::from_configuration(configuration).run()
}

fn first_argument_is_typed_configuration_source() -> bool {
    let Some(argument) = std::env::args_os().nth(1) else {
        return false;
    };
    let lossy = argument.to_string_lossy();
    if lossy.starts_with('(') {
        return true;
    }
    let path = std::path::Path::new(&argument);
    matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("nota") | Some("rkyv")
    )
}
