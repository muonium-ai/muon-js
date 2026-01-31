use std::env;

#[cfg(feature = "mini-redis")]
#[async_std::main]
async fn main() -> std::io::Result<()> {
    let mut bind = "127.0.0.1".to_string();
    let mut port: u16 = 6379;
    let mut databases: usize = 16;
    let mut persist_path: Option<String> = None;
    let mut aof_enabled: bool = false;

    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
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
            _ => {}
        }
    }

    let config = muon_js::mini_redis::server::ServerConfig {
        bind,
        port,
        databases,
        persist_path,
        aof_enabled,
    };
    muon_js::mini_redis::server::run(config).await
}

#[cfg(not(feature = "mini-redis"))]
fn main() {
    eprintln!("mini-redis disabled: rebuild with --features mini-redis");
}
