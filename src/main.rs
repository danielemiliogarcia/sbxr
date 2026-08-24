mod app;
mod auth;
mod cli;
mod embedded;
mod environment;
mod host;
mod process;
mod sandbox;
mod sha256;
mod sync;
mod vscode;

fn main() {
    match app::run() {
        Ok(0) => {}
        Ok(code) => std::process::exit(code),
        Err(message) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
    }
}
