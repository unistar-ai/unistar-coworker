use tracing_subscriber::EnvFilter;

/// Headless / daemon: logs go to stderr. TUI: suppress stderr (ratatui owns the terminal).
pub fn init_tracing(tui_mode: bool) {
    let filter = EnvFilter::from_default_env()
        .add_directive("unistar_coworker=info".parse().expect("valid directive"));

    let builder = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(false)
        .with_target(tui_mode);

    if tui_mode {
        builder.with_writer(std::io::sink).init();
    } else {
        builder.with_writer(std::io::stderr).init();
    }
}
