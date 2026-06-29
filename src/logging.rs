use tracing_subscriber::EnvFilter;

/// Headless / daemon: logs go to stderr. TUI: suppress stderr (ratatui owns the
/// terminal). `suppress_stderr` (interactive chat REPL) also sinks logs so they
/// don't interleave with the in-place reasoning preview / streamed reply.
pub fn init_tracing(tui_mode: bool, verbose: u8, quiet: bool, suppress_stderr: bool) {
    let base = if quiet {
        "warn"
    } else if verbose >= 2 {
        "trace"
    } else if verbose >= 1 {
        "debug"
    } else {
        "info"
    };
    let filter = EnvFilter::from_default_env().add_directive(
        format!("unistar_coworker={base}")
            .parse()
            .expect("valid directive"),
    );
    #[cfg(feature = "web-browser")]
    let filter = filter
        .add_directive("chromiumoxide=warn".parse().expect("valid directive"))
        // conn logs the same parse failure without payload; handler patch includes payload.
        .add_directive(
            "chromiumoxide::conn::raw_ws::parse_errors=error"
                .parse()
                .expect("valid directive"),
        );

    let builder = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(false)
        .with_target(tui_mode);

    if tui_mode || suppress_stderr {
        builder.with_writer(std::io::sink).init();
    } else {
        builder.with_writer(std::io::stderr).init();
    }
}
