fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!(
        "{}",
        serde_json::to_string_pretty(&lens_contract::schema::eval_contract())?
    );
    Ok(())
}
