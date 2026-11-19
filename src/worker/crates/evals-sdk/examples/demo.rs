fn main() -> std::io::Result<()> {
    let fast = std::env::args().any(|arg| arg == "--fast");
    lens_evals_sdk::demo::run(lens_evals_sdk::demo::Options::detect(fast), &mut || Ok(()))
}
