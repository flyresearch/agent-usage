//! Read-only qualification helper: emits numeric projection, never raw records.
use agent_usage::{parser, Tokens};
use std::{collections::BTreeMap, fs::File, io::BufReader};
fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 2 {
        std::process::exit(2);
    }
    let result = File::open(&args[1])
        .map_err(|_| agent_usage::Error("read_failed"))
        .and_then(|f| {
            parser::parse(
                BufReader::new(f),
                args[0].to_str().unwrap_or(""),
                &BTreeMap::new(),
                "sample",
            )
        });
    let value = match result {
        Ok(p) => {
            let total = p
                .records
                .iter()
                .try_fold(Tokens::default(), |a, r| a.checked_add(r.tokens));
            match total {
                Ok(t) => serde_json::json!({"tokens":t,"coverage":p.coverage}),
                Err(e) => serde_json::json!({"error":e.0}),
            }
        }
        Err(e) => serde_json::json!({"error":e.0}),
    };
    println!("{value}");
}
