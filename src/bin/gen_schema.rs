// Emit the JSON schema for the `config.toml` Config struct on stdout.
//
// Run via `cargo run --bin gen_schema > config.schema.json`. CI diffs the
// committed schema against this output to catch struct drift.

use cc_statusline::config::Config;

fn main() {
    let schema = schemars::schema_for!(Config);
    println!("{}", serde_json::to_string_pretty(&schema).unwrap());
}
