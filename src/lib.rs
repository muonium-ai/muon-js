//! Muon JS: a native Rust port of MQuickJS (not a wrapper).

mod api;
mod context;
mod types;
mod value;

pub use api::*;
pub use types::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn array_no_holes() {
        let mut mem = vec![0u8; 4096];
        let mut ctx = JS_NewContext(&mut mem);
        let arr = JS_NewArray(&mut ctx, 0);
        assert!(!arr.is_exception());
        let v1 = JS_NewInt32(&mut ctx, 42);
        let r0 = JS_SetPropertyUint32(&mut ctx, arr, 0, v1);
        assert!(!r0.is_exception());
        let r2 = JS_SetPropertyUint32(&mut ctx, arr, 2, v1);
        assert!(r2.is_exception());
    }

    #[test]
    fn object_property_roundtrip() {
        let mut mem = vec![0u8; 4096];
        let mut ctx = JS_NewContext(&mut mem);
        let obj = JS_NewObject(&mut ctx);
        let v = JS_NewInt32(&mut ctx, 7);
        let r = JS_SetPropertyStr(&mut ctx, obj, "x", v);
        assert!(!r.is_exception());
        let got = JS_GetPropertyStr(&mut ctx, obj, "x");
        assert!(JS_IsNumber(&mut ctx, got) != 0);
        let val = JS_ToInt32(&mut ctx, got).expect("int32");
        assert_eq!(val, 7);
    }

    #[test]
    fn array_length_rules() {
        let mut mem = vec![0u8; 4096];
        let mut ctx = JS_NewContext(&mut mem);
        let arr = JS_NewArray(&mut ctx, 0);
        let zero = JS_NewInt32(&mut ctx, 0);
        let ok = JS_SetPropertyStr(&mut ctx, arr, "length", zero);
        assert!(!ok.is_exception());
        let one = JS_NewInt32(&mut ctx, 1);
        let bad = JS_SetPropertyStr(&mut ctx, arr, "length", one);
        assert!(bad.is_exception());
        let five = JS_NewInt32(&mut ctx, 5);
        let _ = JS_SetPropertyUint32(&mut ctx, arr, 0, five);
        let shrink = JS_SetPropertyStr(&mut ctx, arr, "length", zero);
        assert!(!shrink.is_exception());
    }
}
