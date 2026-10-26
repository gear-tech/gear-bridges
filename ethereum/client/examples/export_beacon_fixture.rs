//! Test-only offline export: export_beacon_fixture --input SPEC.json --output BUNDLE.json
//! Build explicitly with: cargo build -p ethereum-client --features test-fixtures --example export_beacon_fixture
//! Never install this example as a production relayer or a source of public-network trust.
use anyhow::{bail, ensure, Context, Result};
use std::{fs, path::PathBuf};

fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let (mut input, mut output) = (None, None);
    while let Some(flag) = args.next() {
        let destination = if flag == "--input" {
            &mut input
        } else if flag == "--output" {
            &mut output
        } else {
            bail!("usage: export_beacon_fixture --input SPEC.json --output BUNDLE.json")
        };
        ensure!(destination.is_none(), "duplicate exporter argument");
        *destination = Some(PathBuf::from(args.next().context("missing argument path")?));
    }
    let input = input.context("--input is required")?;
    let output = output.context("--output is required")?;
    ensure!(
        fs::metadata(&input)?.len() <= 16 * 1024 * 1024,
        "scenario input exceeds 16 MiB"
    );
    let bytes = fs::read(input)?;
    // Full SSZ container types use the same bounded large stack as the native Beacon tests.
    let hash = std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?
                .block_on(
                    ethereum_client::beacon_finality::test_fixtures::export_scenario(
                        &bytes, output,
                    ),
                )
        })?
        .join()
        .map_err(|_| anyhow::anyhow!("fixture exporter thread panicked"))??;
    println!("{hash:#x}");
    Ok(())
}
