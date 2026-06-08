use system::SystemDaemon;
use system::schema::daemon::DaemonEntry;

fn main() -> std::process::ExitCode {
    <SystemDaemon as DaemonEntry>::run_to_exit_code()
}
