use clap::Parser;

fn main() {
    env_logger::init();

    let args = nscb::cli::Args::parse();

    if let Err(e) = nscb::cli::dispatch(args) {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}
