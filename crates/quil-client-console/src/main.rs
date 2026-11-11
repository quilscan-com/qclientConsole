fn main() {
    std::process::exit(quil_client_console::launcher::run_process_from(
        std::env::args_os(),
    ));
}
