// Release builds on Windows must not open a console window next to the app.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod desktop;
mod formatter;
mod profile;
mod routes;
mod runtime;
mod workspace;

use std::process::ExitCode;

fn run(arguments: Vec<std::ffi::OsString>) -> Result<(), String> {
    if arguments == [formatter::FLAG] {
        return formatter::child_main();
    }
    let path = match profile::select(arguments.iter().cloned(), profile::default_root) {
        Ok(path) => path,
        Err(profile::Error::InvalidArguments) => {
            return Err("Usage: rsql-desktop [--profile-dir PATH]".into());
        }
        Err(profile::Error::MissingDataDirectory) => {
            let message = "No application data directory. Specify --profile-dir PATH.";
            eprintln!("{}: {message}", profile::NAME);
            return app::run(Err(message.into()), arguments, None);
        }
    };
    let runtime = tokio::runtime::Runtime::new().map_err(|e| format!("Start desktop runtime: {e}"));
    let session = match &runtime {
        Ok(runtime) => runtime::Session::open(runtime.handle().clone(), &path),
        Err(message) => Err(message.clone()),
    };
    if let Err(message) = &session {
        eprintln!("{}: {message}", profile::NAME);
    }
    let status = session.as_ref().ok().map(|s| s.save_status());
    let recovery = runtime.as_ref().ok().map(|rt| (rt.handle().clone(), path));
    let result = app::run(session, arguments, recovery);
    if let Some(error) = status.and_then(|s| s.borrow().clone()) {
        return Err(error);
    }
    result
}

fn main() -> ExitCode {
    let arguments = std::env::args_os().skip(1).collect::<Vec<_>>();
    if let Some(status) = rsql_desktop::update::dispatch_resolver(&arguments) {
        return status;
    }
    if let Some(status) = rsql_desktop::migration::cli::dispatch(&arguments) {
        return status;
    }
    match run(arguments) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{}: {error}", profile::NAME);
            ExitCode::FAILURE
        }
    }
}
