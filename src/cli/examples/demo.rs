fn main() -> std::io::Result<()> {
    let fast = std::env::args().any(|arg| arg == "--fast");
    lens_cli::demo::run(lens_cli::demo::Options::detect(fast), &mut || Ok(()))
}
