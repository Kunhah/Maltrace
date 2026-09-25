//! Standalone HTTP server binary — the "web" half of the dual-target build.
//!
//! Run from the `app/` directory so the relative path to `../dist` (Vite's
//! production output) resolves correctly:
//!
//! ```text
//! cargo run --bin maltrace-server -- --host 127.0.0.1 --port 1421 --static ../dist
//! ```
//!
//! All real work lives in the library crate; this is just argument parsing.

use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut host: IpAddr = "127.0.0.1".parse()?;
    let mut port: u16 = 1421;
    // Default: serve ../dist if it exists (i.e. user ran `npm run build` first).
    let mut static_dir: Option<PathBuf> = {
        let p = PathBuf::from("../dist");
        if p.exists() { Some(p) } else { None }
    };
    let mut api_only = false;
    let mut encrypt = false;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--host" => host = args.next().expect("--host needs a value").parse()?,
            "--port" => port = args.next().expect("--port needs a value").parse()?,
            "--static" => static_dir = Some(PathBuf::from(args.next().expect("--static needs a path"))),
            "--api-only" => api_only = true,
            // One-shot: encrypt the on-disk DB with the passphrase from
            // MALTRACE_DB_PASSPHRASE and exit. The server does NOT start
            // after migration — restart with the env var set so the new
            // connection unlocks the encrypted DB.
            "--encrypt-db" => encrypt = true,
            "-h" | "--help" => {
                println!(
                    "maltrace-server [--host IP] [--port N] [--static DIR] [--api-only] [--encrypt-db]\n\n\
                     Defaults: host=127.0.0.1, port=1421, static=../dist (if it exists).\n\n\
                     --encrypt-db: one-shot migration of the on-disk SQLite DB to\n\
                       SQLCipher format using the MALTRACE_DB_PASSPHRASE env var.\n\
                       Doesn't start the server."
                );
                return Ok(());
            }
            other => anyhow::bail!("unknown argument: {other}"),
        }
    }
    if api_only {
        static_dir = None;
    }

    if encrypt {
        let pp = std::env::var("MALTRACE_DB_PASSPHRASE")
            .map_err(|_| anyhow::anyhow!("--encrypt-db requires MALTRACE_DB_PASSPHRASE in the environment"))?;
        let db_path = app_lib::server_data_dir().join("maltrace.sqlite");
        eprintln!("maltrace: encrypting {}…", db_path.display());
        app_lib::encrypt_db_at(&db_path, &pp)?;
        eprintln!("maltrace: done. Restart with the same MALTRACE_DB_PASSPHRASE.");
        return Ok(());
    }

    app_lib::run_server(SocketAddr::new(host, port), static_dir).await
}
