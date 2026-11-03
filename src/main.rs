mod app;
mod auth;
mod browser;
mod cli;
mod diagnostics;
mod embedded;
mod envfile;
mod environment;
mod git;
mod github_proxy;
mod host;
mod node_tools;
mod orca;
mod process;
mod profile;
mod sandbox;
mod sbx;
mod sha256;
mod sign;
mod sync;
mod verify;
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

#[cfg(test)]
mod tests {
    #[test]
    fn release_version_file_matches_cargo_package() {
        assert_eq!(include_str!("../VERSION").trim(), env!("CARGO_PKG_VERSION"));
    }
}
