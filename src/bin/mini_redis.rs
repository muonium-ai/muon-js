use std::env;

#[cfg(feature = "mini-redis")]
#[async_std::main]
async fn main() -> std::io::Result<()> {
    let mut bind = "127.0.0.1".to_string();
    let mut port: u16 = 6379;
    let mut databases: usize = 16;

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
            _ => {}
        }
    }

    let config = muon_js::mini_redis::server::ServerConfig { bind, port, databases };
    muon_js::mini_redis::server::run(config).await
}

#[cfg(not(feature = "mini-redis"))]
fn main() {
    eprintln!("mini-redis disabled: rebuild with --features mini-redis");
}
