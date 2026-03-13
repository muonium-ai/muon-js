#[cfg(feature = "muoncache")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[cfg(feature = "muoncache")]
const MUONCACHE_VERSION: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/MUONCACHE_VERSION"));
#[cfg(feature = "muoncache")]
const CARGO_VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(feature = "muoncache")]
#[tokio::main]
async fn main() -> std::io::Result<()> {
    use std::env;
    let mut bind = "127.0.0.1".to_string();
    let mut port: u16 = 6379;
    let mut databases: usize = 16;
    let mut persist_path: Option<String> = None;
    let mut aof_enabled: bool = false;
    let mut script_mem: Option<usize> = None;
    let mut script_reset_threshold: Option<u8> = None;

    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--version" | "-V" => {
                println!("muoncache {}", MUONCACHE_VERSION.trim());
                println!("muonjs {}", muon_js::MUONJS_VERSION.trim());
                println!("cargo {}", CARGO_VERSION);
                return Ok(());
            }
            "--bind" => {
                if let Some(v) = args.next() {
                    bind = v;
                }
            }
            "--port" => {
                if let Some(v) = args.next() {
                    if let Ok(p) = v.parse::<u16>() {
                        port = p;
                    }
                }
            }
            "--databases" => {
                if let Some(v) = args.next() {
                    if let Ok(d) = v.parse::<usize>() {
                        databases = d.max(1);
                    }
                }
            }
            "--persist" => {
                if let Some(v) = args.next() {
                    persist_path = Some(v);
                }
            }
            "--aof" => {
                aof_enabled = true;
            }
            "--script-mem" => {
                if let Some(v) = args.next() {
                    if let Ok(bytes) = v.parse::<usize>() {
                        script_mem = Some(bytes);
                    }
                }
            }
            "--script-reset-threshold" => {
                if let Some(v) = args.next() {
                    if let Ok(pct) = v.parse::<u8>() {
                        script_reset_threshold = Some(pct);
                    }
                }
            }
            _ => {}
        }
    }

    let mut script_runtime = muon_js::muon_cache::server::ScriptRuntimeConfig::default();
    if let Some(mem) = script_mem {
        script_runtime.mem_size = mem;
    }
    if let Some(pct) = script_reset_threshold {
        script_runtime.reset_threshold_pct = pct;
    }

    let config = muon_js::muon_cache::server::ServerConfig {
        bind,
        port,
        databases,
        persist_path,
        aof_enabled,
        script_runtime,
    };
    muon_js::muon_cache::server::run(config).await
}

#[cfg(not(feature = "muoncache"))]
fn main() {
    eprintln!("muoncache disabled: rebuild with --features muoncache");
}
