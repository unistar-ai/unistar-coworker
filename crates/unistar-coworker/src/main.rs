use coworker_cli::{err_prefix, run};

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("{} {e}", err_prefix());
        std::process::exit(coworker_core::exit_codes::exit_code_for_error(&e));
    }
}
