use muon_js::{JS_EVAL_RETVAL, JS_Eval, JS_NewContext, JS_Parse, JS_Run, JSValue};

const BASIC_SAMPLES: &[(&str, &str)] = &[
    ("01_arithmetic.js", include_str!("../samples/pass/01_arithmetic.js")),
    ("02_variables.js", include_str!("../samples/pass/02_variables.js")),
    ("03_strings.js", include_str!("../samples/pass/03_strings.js")),
    ("04_null_undefined.js", include_str!("../samples/pass/04_null_undefined.js")),
    ("05_arrays_simple.js", include_str!("../samples/pass/05_arrays_simple.js")),
    ("06_objects.js", include_str!("../samples/pass/06_objects.js")),
];

const FEATURE_SAMPLES: &[(&str, &str)] = &[
    ("01_array_indexing.js", include_str!("../samples/fail/01_array_indexing.js")),
    ("02_functions.js", include_str!("../samples/fail/02_functions.js")),
    ("03_property_access.js", include_str!("../samples/fail/03_property_access.js")),
];

fn eval_sample(source: &str, filename: &str) -> JSValue {
    let mut mem = vec![0u8; 4096];
    let mut ctx = JS_NewContext(&mut mem);
    JS_Eval(&mut ctx, source, filename, JS_EVAL_RETVAL)
}

fn parse_and_run_sample(source: &str, filename: &str) -> JSValue {
    let mut mem = vec![0u8; 4096];
    let mut ctx = JS_NewContext(&mut mem);
    let parsed = JS_Parse(&mut ctx, source, filename, JS_EVAL_RETVAL);
    if parsed.is_exception() {
        return parsed;
    }
    JS_Run(&mut ctx, parsed)
}

#[test]
fn eval_samples() {
    for (name, src) in BASIC_SAMPLES {
        let val = eval_sample(src, name);
        assert!(!val.is_exception(), "eval failed for {name}");
    }
}

#[test]
fn parse_and_run_samples() {
    for (name, src) in BASIC_SAMPLES {
        let val = parse_and_run_sample(src, name);
        assert!(!val.is_exception(), "parse/run failed for {name}");
    }
}

#[test]
#[ignore = "pending feature support"]
fn eval_feature_samples_expected_to_fail() {
    for (name, src) in FEATURE_SAMPLES {
        let val = eval_sample(src, name);
        assert!(val.is_exception(), "expected failure for {name}");
    }
}

#[test]
#[ignore = "pending feature support"]
fn parse_and_run_feature_samples_expected_to_fail() {
    for (name, src) in FEATURE_SAMPLES {
        let val = parse_and_run_sample(src, name);
        assert!(val.is_exception(), "expected failure for {name}");
    }
}
