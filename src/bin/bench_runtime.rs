use muon_js::{
    Compiler, JSCStringBuf, JS_EVAL_RETVAL, JS_Eval, JS_GetException, JS_NewContext, JS_ToCString,
    JS_ToString, JSValue, VM,
};
use std::env;
use std::fs;
use std::path::PathBuf;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

struct BenchConfig {
    iterations: usize,
    warmup: usize,
    runs: usize,
    out: Option<PathBuf>,
}

struct CaseResult {
    name: &'static str,
    iterations: usize,
    runs: usize,
    warmup: usize,
    median_seconds: f64,
    mean_seconds: f64,
    median_ops_per_sec: f64,
    mean_ops_per_sec: f64,
}

fn parse_args() -> BenchConfig {
    let mut iterations = 5000usize;
    let mut warmup = 500usize;
    let mut runs = 5usize;
    let mut out = None;

    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--iterations" => {
                if let Some(v) = args.next() {
                    iterations = v.parse().unwrap_or(iterations);
                }
            }
            "--warmup" => {
                if let Some(v) = args.next() {
                    warmup = v.parse().unwrap_or(warmup);
                }
            }
            "--runs" => {
                if let Some(v) = args.next() {
                    runs = v.parse().unwrap_or(runs);
                }
            }
            "--out" => {
                if let Some(v) = args.next() {
                    out = Some(PathBuf::from(v));
                }
            }
            _ => {}
        }
    }

    BenchConfig {
        iterations,
        warmup,
        runs,
        out,
    }
}

fn eval_checked(ctx: &mut muon_js::JSContextImpl, source: &str) -> JSValue {
    let val = JS_Eval(ctx, source, "<bench>", JS_EVAL_RETVAL);
    if val.is_exception() {
        let exc = JS_GetException(ctx);
        let exc_str = JS_ToString(ctx, exc);
        let mut buf = JSCStringBuf { buf: [0u8; 5] };
        let msg = JS_ToCString(ctx, exc_str, &mut buf);
        panic!("benchmark script exception: {msg}");
    }
    val
}

fn run_case<F>(name: &'static str, config: &BenchConfig, mut source_for_iter: F) -> CaseResult
where
    F: FnMut(usize) -> String,
{
    let mut per_run_seconds = Vec::with_capacity(config.runs);

    for run_index in 0..config.runs {
        let mut mem = vec![0u8; 64 * 1024 * 1024];
        let mut ctx = JS_NewContext(&mut mem);

        for i in 0..config.warmup {
            let src = source_for_iter(i + run_index * config.warmup);
            let _ = eval_checked(&mut ctx, &src);
        }

        let start = Instant::now();
        for i in 0..config.iterations {
            let src = source_for_iter(i + run_index * config.iterations);
            let _ = eval_checked(&mut ctx, &src);
        }
        let elapsed = start.elapsed().as_secs_f64();
        per_run_seconds.push(elapsed);
    }

    let mut sorted = per_run_seconds.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median_seconds = if sorted.is_empty() {
        0.0
    } else if sorted.len() % 2 == 1 {
        sorted[sorted.len() / 2]
    } else {
        let i = sorted.len() / 2;
        (sorted[i - 1] + sorted[i]) / 2.0
    };

    let mean_seconds = if per_run_seconds.is_empty() {
        0.0
    } else {
        per_run_seconds.iter().sum::<f64>() / per_run_seconds.len() as f64
    };

    let median_ops_per_sec = if median_seconds > 0.0 {
        config.iterations as f64 / median_seconds
    } else {
        0.0
    };
    let mean_ops_per_sec = if mean_seconds > 0.0 {
        config.iterations as f64 / mean_seconds
    } else {
        0.0
    };

    CaseResult {
        name,
        iterations: config.iterations,
        runs: config.runs,
        warmup: config.warmup,
        median_seconds,
        mean_seconds,
        median_ops_per_sec,
        mean_ops_per_sec,
    }
}

fn run_vm_global_case(config: &BenchConfig) -> CaseResult {
    let mut per_run_seconds = Vec::with_capacity(config.runs);

    for _ in 0..config.runs {
        let mut mem = vec![0u8; 64 * 1024 * 1024];
        let mut ctx = JS_NewContext(&mut mem);
        let _ = eval_checked(&mut ctx, "var x = 0; x");

        let mut compiler = Compiler::new();
        let module = compiler
            .compile_program(&mut ctx, "x = x + 1")
            .expect("failed to compile VM benchmark program");
        let mut vm = VM::new();

        for _ in 0..config.warmup {
            let _ = vm.run_module(&mut ctx, &module);
        }

        let start = Instant::now();
        for _ in 0..config.iterations {
            let _ = vm.run_module(&mut ctx, &module);
        }
        let elapsed = start.elapsed().as_secs_f64();
        per_run_seconds.push(elapsed);
    }

    let mut sorted = per_run_seconds.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median_seconds = if sorted.is_empty() {
        0.0
    } else if sorted.len() % 2 == 1 {
        sorted[sorted.len() / 2]
    } else {
        let i = sorted.len() / 2;
        (sorted[i - 1] + sorted[i]) / 2.0
    };
    let mean_seconds = if per_run_seconds.is_empty() {
        0.0
    } else {
        per_run_seconds.iter().sum::<f64>() / per_run_seconds.len() as f64
    };
    let median_ops_per_sec = if median_seconds > 0.0 {
        config.iterations as f64 / median_seconds
    } else {
        0.0
    };
    let mean_ops_per_sec = if mean_seconds > 0.0 {
        config.iterations as f64 / mean_seconds
    } else {
        0.0
    };

    CaseResult {
        name: "vm_global_load_store",
        iterations: config.iterations,
        runs: config.runs,
        warmup: config.warmup,
        median_seconds,
        mean_seconds,
        median_ops_per_sec,
        mean_ops_per_sec,
    }
}

fn default_output_path() -> PathBuf {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    PathBuf::from(format!("tmp/comparison/js_runtime_benchmark_{ts}.json"))
}

fn write_json(path: &PathBuf, results: &[CaseResult], config: &BenchConfig) {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    let mut json = String::new();
    json.push_str("{\n");
    json.push_str("  \"suite\": \"js-runtime-microbench\",\n");
    json.push_str(&format!("  \"iterations\": {},\n", config.iterations));
    json.push_str(&format!("  \"warmup\": {},\n", config.warmup));
    json.push_str(&format!("  \"runs\": {},\n", config.runs));
    json.push_str("  \"results\": [\n");

    for (idx, result) in results.iter().enumerate() {
        json.push_str("    {\n");
        json.push_str(&format!("      \"name\": \"{}\",\n", result.name));
        json.push_str(&format!("      \"iterations\": {},\n", result.iterations));
        json.push_str(&format!("      \"runs\": {},\n", result.runs));
        json.push_str(&format!("      \"warmup\": {},\n", result.warmup));
        json.push_str(&format!("      \"median_seconds\": {:.6},\n", result.median_seconds));
        json.push_str(&format!("      \"mean_seconds\": {:.6},\n", result.mean_seconds));
        json.push_str(&format!(
            "      \"median_ops_per_sec\": {:.2},\n",
            result.median_ops_per_sec
        ));
        json.push_str(&format!("      \"mean_ops_per_sec\": {:.2}\n", result.mean_ops_per_sec));
        json.push_str("    }");
        if idx + 1 != results.len() {
            json.push(',');
        }
        json.push('\n');
    }

    json.push_str("  ]\n");
    json.push_str("}\n");

    fs::write(path, json).expect("failed to write benchmark output");
}

fn main() {
    let config = parse_args();

    let cases: Vec<CaseResult> = vec![
        run_case("parser_arithmetic", &config, |i| {
            let a = (i % 1000) as i32;
            let b = ((i * 3) % 997) as i32;
            let c = ((i * 7) % 983) as i32;
            format!("({a} + {b}) * {c} - {a}")
        }),
        run_case("eval_for_loop", &config, |_| {
            "var s=0; for (var i=0; i<50; i++) { s = s + i; } s".to_string()
        }),
        run_case("global_property_roundtrip", &config, |_| {
            "globalThis.counter = (globalThis.counter || 0) + 1; globalThis.counter".to_string()
        }),
        run_vm_global_case(&config),
        run_case("string_replace_all", &config, |_| {
            "\"abcabcabcabc\".replaceAll(\"ab\", \"xy\")".to_string()
        }),
        run_case("string_replace_regex", &config, |_| {
            "\"a1b2c3d4\".replace(/[0-9]/g, \"x\")".to_string()
        }),
        run_case("object_property_access", &config, |_| {
            "var o={a:1,b:2,c:3,d:4}; o.a + o.b + o.c + o.d".to_string()
        }),
    ];

    println!("JS runtime microbench (iterations={}, warmup={}, runs={})", config.iterations, config.warmup, config.runs);
    println!("{:<28} {:>12} {:>12}", "CASE", "MED_OPS/S", "MEAN_OPS/S");
    println!("{}", "-".repeat(56));
    for c in &cases {
        println!(
            "{:<28} {:>12.2} {:>12.2}",
            c.name, c.median_ops_per_sec, c.mean_ops_per_sec
        );
    }

    let out_path = config.out.clone().unwrap_or_else(default_output_path);
    write_json(&out_path, &cases, &config);
    println!("OUTPUT_JSON={}", out_path.display());
}
