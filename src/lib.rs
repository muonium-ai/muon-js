//! Muon JS: a native Rust port of MQuickJS (not a wrapper).

mod helpers;
mod json;
mod api;
mod context;
mod types;
mod value;

pub use api::*;
pub use types::*;

#[cfg(test)]
mod tests {
    use super::*;

    fn eval_ret(ctx: &mut JSContextImpl, src: &str) -> JSValue {
        JS_Eval(ctx, src, "test.js", JS_EVAL_RETVAL)
    }

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

    #[test]
    fn to_string_primitives() {
        let mut mem = vec![0u8; 4096];
        let mut ctx = JS_NewContext(&mut mem);
        let mut buf = JSCStringBuf { buf: [0u8; 5] };
        let t = JSValue::new_bool(true);
        let ts = JS_ToString(&mut ctx, t);
        let s = JS_ToCString(&mut ctx, ts, &mut buf);
        assert_eq!(s, "true");
        let u = JSValue::UNDEFINED;
        let us = JS_ToString(&mut ctx, u);
        let su = JS_ToCString(&mut ctx, us, &mut buf);
        assert_eq!(su, "undefined");
    }

    #[test]
    fn to_number_strings() {
        let mut mem = vec![0u8; 4096];
        let mut ctx = JS_NewContext(&mut mem);
        let s = JS_NewString(&mut ctx, "  -3.5 ");
        let v = JS_ToNumber(&mut ctx, s).unwrap();
        assert!((v + 3.5).abs() < 1e-9);
        let hex = JS_NewString(&mut ctx, "0x10");
        let hv = JS_ToNumber(&mut ctx, hex).unwrap();
        assert!((hv - 16.0).abs() < 1e-9);
        let s_int = JS_NewString(&mut ctx, "42");
        let iv = JS_ToInt32(&mut ctx, s_int).unwrap();
        assert_eq!(iv, 42);
        let s_nan = JS_NewString(&mut ctx, "NaN");
        let nv = JS_ToInt32(&mut ctx, s_nan).unwrap();
        assert_eq!(nv, 0);
        let undef = JSValue::UNDEFINED;
        let uv = JS_ToNumber(&mut ctx, undef).unwrap();
        assert!(uv.is_nan());
    }

    #[test]
    fn opaque_roundtrip() {
        let mut mem = vec![0u8; 4096];
        let mut ctx = JS_NewContext(&mut mem);
        let obj = JS_NewObject(&mut ctx);
        let ptr = 0x1234usize as *mut core::ffi::c_void;
        JS_SetOpaque(&mut ctx, obj, ptr);
        let got = JS_GetOpaque(&mut ctx, obj);
        assert_eq!(got, ptr);
    }

    #[test]
    fn bytecode_magic_check() {
        let magic = JS_BYTECODE_MAGIC.to_ne_bytes();
        let mut buf = [magic[0], magic[1], 0, 0];
        assert_eq!(JS_IsBytecode(&buf), 1);
        let mut mem = vec![0u8; 64];
        let mut ctx = JS_NewContext(&mut mem);
        assert_eq!(JS_RelocateBytecode(&mut ctx, &mut buf), 0);
        let bad = [0u8; 4];
        assert_eq!(JS_IsBytecode(&bad), 0);
    }

    #[test]
    fn global_object_property() {
        let mut mem = vec![0u8; 4096];
        let mut ctx = JS_NewContext(&mut mem);
        let global = JS_GetGlobalObject(&mut ctx);
        let v = JS_NewInt32(&mut ctx, 99);
        let r = JS_SetPropertyStr(&mut ctx, global, "g", v);
        assert!(!r.is_exception());
        let got = JS_GetPropertyStr(&mut ctx, global, "g");
        let val = JS_ToInt32(&mut ctx, got).expect("int32");
        assert_eq!(val, 99);
    }

    #[test]
    fn float_roundtrip() {
        let mut mem = vec![0u8; 4096];
        let mut ctx = JS_NewContext(&mut mem);
        let f = JS_NewFloat64(&mut ctx, 1.5);
        assert_eq!(JS_IsNumber(&mut ctx, f), 1);
        let n = JS_ToNumber(&mut ctx, f).expect("number");
        assert!((n - 1.5).abs() < 1e-9);
        let mut buf = JSCStringBuf { buf: [0u8; 5] };
        let fs = JS_ToString(&mut ctx, f);
        let s = JS_ToCString(&mut ctx, fs, &mut buf);
        assert!(s.starts_with('1'));
    }

    #[test]
    fn large_int_conversions() {
        let mut mem = vec![0u8; 4096];
        let mut ctx = JS_NewContext(&mut mem);
        let u = JS_NewUint32(&mut ctx, u32::MAX);
        let nu = JS_ToNumber(&mut ctx, u).expect("number");
        assert!((nu - 4_294_967_295.0).abs() < 1.0);
        let i = JS_NewInt64(&mut ctx, 1_i64 << 40);
        let ni = JS_ToNumber(&mut ctx, i).expect("number");
        assert!(ni > 1.0e12);
    }

    #[test]
    fn c_function_object() {
        let mut mem = vec![0u8; 4096];
        let mut ctx = JS_NewContext(&mut mem);
        let func = JS_NewCFunctionParams(&mut ctx, 1, JSValue::UNDEFINED);
        assert_eq!(JS_IsFunction(&mut ctx, func), 1);
    }

    #[test]
    fn c_function_table_dispatch() {
        fn add_one(
            ctx: *mut JSContext,
            _this_val: *mut JSValue,
            argc: i32,
            argv: *mut JSValue,
        ) -> JSValue {
            if argc < 1 {
                return JSValue::EXCEPTION;
            }
            let ctx = unsafe { &mut *(ctx as *mut JSContextImpl) };
            let arg0 = unsafe { *argv };
            if let Ok(v) = js_to_int32(ctx, arg0) {
                return js_new_int32(ctx, v + 1);
            }
            JSValue::EXCEPTION
        }

        let mut mem = vec![0u8; 4096];
        let mut ctx = JS_NewContext(&mut mem);
        let def = JSCFunctionDef {
            func: JSCFunctionType { generic: Some(add_one) },
            name: JSValue::UNDEFINED,
            def_type: JSCFunctionDefEnum::Generic as u8,
            arg_count: 1,
            magic: 0,
        };
        let table = [def];
        JS_SetCFunctionTable(&mut ctx, &table);
        let func = JS_NewCFunctionParams(&mut ctx, 0, JSValue::UNDEFINED);
        let two = JS_NewInt32(&mut ctx, 2);
        JS_PushArg(&mut ctx, two);
        JS_PushArg(&mut ctx, func);
        JS_PushArg(&mut ctx, JSValue::UNDEFINED);
        let res = JS_Call(&mut ctx, 1);
        assert_eq!(JS_ToInt32(&mut ctx, res).unwrap(), 3);
    }

    #[test]
    fn c_function_magic_and_params() {
        fn add_magic(
            ctx: *mut JSContext,
            _this_val: *mut JSValue,
            argc: i32,
            argv: *mut JSValue,
            magic: i32,
        ) -> JSValue {
            let ctx = unsafe { &mut *(ctx as *mut JSContextImpl) };
            if argc < 1 {
                return JSValue::EXCEPTION;
            }
            let arg0 = unsafe { *argv };
            if let Ok(v) = js_to_int32(ctx, arg0) {
                return js_new_int32(ctx, v + magic);
            }
            JSValue::EXCEPTION
        }

        fn add_params(
            ctx: *mut JSContext,
            _this_val: *mut JSValue,
            argc: i32,
            argv: *mut JSValue,
            params: JSValue,
        ) -> JSValue {
            let ctx = unsafe { &mut *(ctx as *mut JSContextImpl) };
            if argc < 1 {
                return JSValue::EXCEPTION;
            }
            let arg0 = unsafe { *argv };
            let base = js_to_int32(ctx, arg0).unwrap_or(0);
            let inc = js_to_int32(ctx, params).unwrap_or(0);
            js_new_int32(ctx, base + inc)
        }

        let mut mem = vec![0u8; 4096];
        let mut ctx = JS_NewContext(&mut mem);
        let def_magic = JSCFunctionDef {
            func: JSCFunctionType { generic_magic: Some(add_magic) },
            name: JSValue::UNDEFINED,
            def_type: JSCFunctionDefEnum::GenericMagic as u8,
            arg_count: 1,
            magic: 5,
        };
        let def_params = JSCFunctionDef {
            func: JSCFunctionType { generic_params: Some(add_params) },
            name: JSValue::UNDEFINED,
            def_type: JSCFunctionDefEnum::GenericParams as u8,
            arg_count: 1,
            magic: 0,
        };
        let table = [def_magic, def_params];
        JS_SetCFunctionTable(&mut ctx, &table);

        let f_magic = JS_NewCFunctionParams(&mut ctx, 0, JSValue::UNDEFINED);
        let arg = JS_NewInt32(&mut ctx, 1);
        JS_PushArg(&mut ctx, arg);
        JS_PushArg(&mut ctx, f_magic);
        JS_PushArg(&mut ctx, JSValue::UNDEFINED);
        let res = JS_Call(&mut ctx, 1);
        assert_eq!(JS_ToInt32(&mut ctx, res).unwrap(), 6);

        let inc = JS_NewInt32(&mut ctx, 7);
        let f_params = JS_NewCFunctionParams(&mut ctx, 1, inc);
        let arg2 = JS_NewInt32(&mut ctx, 2);
        JS_PushArg(&mut ctx, arg2);
        JS_PushArg(&mut ctx, f_params);
        JS_PushArg(&mut ctx, JSValue::UNDEFINED);
        let res2 = JS_Call(&mut ctx, 1);
        assert_eq!(JS_ToInt32(&mut ctx, res2).unwrap(), 9);
    }

    #[test]
    fn method_call_sets_this() {
        fn return_this(
            _ctx: *mut JSContext,
            this_val: *mut JSValue,
            _argc: i32,
            _argv: *mut JSValue,
        ) -> JSValue {
            unsafe { *this_val }
        }

        let mut mem = vec![0u8; 4096];
        let mut ctx = JS_NewContext(&mut mem);
        let def = JSCFunctionDef {
            func: JSCFunctionType { generic: Some(return_this) },
            name: JSValue::UNDEFINED,
            def_type: JSCFunctionDefEnum::Generic as u8,
            arg_count: 0,
            magic: 0,
        };
        let table = [def];
        JS_SetCFunctionTable(&mut ctx, &table);
        let func = JS_NewCFunctionParams(&mut ctx, 0, JSValue::UNDEFINED);
        let obj = JS_NewObject(&mut ctx);
        let _ = JS_SetPropertyStr(&mut ctx, obj, "f", func);
        let _ = JS_Eval(&mut ctx, "obj = {}", "test.js", 0);
        let global_obj = eval_ret(&mut ctx, "obj");
        let _ = JS_SetPropertyStr(&mut ctx, global_obj, "f", func);
        let res = eval_ret(&mut ctx, "obj.f()");
        assert_eq!(res, global_obj);
    }

    #[test]
    fn bracket_call_sets_this() {
        fn return_this(
            _ctx: *mut JSContext,
            this_val: *mut JSValue,
            _argc: i32,
            _argv: *mut JSValue,
        ) -> JSValue {
            unsafe { *this_val }
        }

        let mut mem = vec![0u8; 4096];
        let mut ctx = JS_NewContext(&mut mem);
        let def = JSCFunctionDef {
            func: JSCFunctionType { generic: Some(return_this) },
            name: JSValue::UNDEFINED,
            def_type: JSCFunctionDefEnum::Generic as u8,
            arg_count: 0,
            magic: 0,
        };
        let table = [def];
        JS_SetCFunctionTable(&mut ctx, &table);
        let func = JS_NewCFunctionParams(&mut ctx, 0, JSValue::UNDEFINED);
        let _ = JS_Eval(&mut ctx, "obj = {}", "test.js", 0);
        let obj = eval_ret(&mut ctx, "obj");
        let _ = JS_SetPropertyStr(&mut ctx, obj, "f", func);
        let res = eval_ret(&mut ctx, "obj[\"f\"]()");
        assert_eq!(res, obj);
    }

    #[test]
    fn register_global_function_helper() {
        fn add_two(
            ctx: *mut JSContext,
            _this_val: *mut JSValue,
            argc: i32,
            argv: *mut JSValue,
        ) -> JSValue {
            if argc < 1 {
                return JSValue::EXCEPTION;
            }
            let ctx = unsafe { &mut *(ctx as *mut JSContextImpl) };
            let arg0 = unsafe { *argv };
            if let Ok(v) = js_to_int32(ctx, arg0) {
                return js_new_int32(ctx, v + 2);
            }
            JSValue::EXCEPTION
        }

        let mut mem = vec![0u8; 4096];
        let mut ctx = JS_NewContext(&mut mem);
        let def = JSCFunctionDef {
            func: JSCFunctionType { generic: Some(add_two) },
            name: JSValue::UNDEFINED,
            def_type: JSCFunctionDefEnum::Generic as u8,
            arg_count: 1,
            magic: 0,
        };
        let table = [def];
        JS_SetCFunctionTable(&mut ctx, &table);
        let _ = JS_RegisterGlobalFunction(&mut ctx, "addTwo", 0, JSValue::UNDEFINED);
        let res = eval_ret(&mut ctx, "addTwo(3)");
        assert_eq!(JS_ToInt32(&mut ctx, res).unwrap(), 5);
    }

    #[test]
    fn register_stdlib_minimal() {
        fn object_ctor(
            ctx: *mut JSContext,
            _this_val: *mut JSValue,
            _argc: i32,
            _argv: *mut JSValue,
        ) -> JSValue {
            let ctx = unsafe { &mut *(ctx as *mut JSContextImpl) };
            js_new_object(ctx)
        }

        fn array_ctor(
            ctx: *mut JSContext,
            _this_val: *mut JSValue,
            argc: i32,
            argv: *mut JSValue,
        ) -> JSValue {
            let ctx = unsafe { &mut *(ctx as *mut JSContextImpl) };
            if argc == 1 {
                let len = js_to_int32(ctx, unsafe { *argv }).unwrap_or(0);
                return js_new_array(ctx, len);
            }
            let arr = js_new_array(ctx, argc);
            if arr.is_exception() {
                return arr;
            }
            for i in 0..argc {
                let v = unsafe { *argv.add(i as usize) };
                let _ = js_set_property_uint32(ctx, arr, i as u32, v);
            }
            arr
        }

        fn object_keys(
            ctx: *mut JSContext,
            _this_val: *mut JSValue,
            argc: i32,
            argv: *mut JSValue,
        ) -> JSValue {
            if argc < 1 {
                return JSValue::EXCEPTION;
            }
            let ctx = unsafe { &mut *(ctx as *mut JSContextImpl) };
            let obj = unsafe { *argv };
            js_object_keys(ctx, obj)
        }

        fn array_is_array(
            ctx: *mut JSContext,
            _this_val: *mut JSValue,
            argc: i32,
            argv: *mut JSValue,
        ) -> JSValue {
            let ctx = unsafe { &mut *(ctx as *mut JSContextImpl) };
            if argc < 1 {
                return JSValue::FALSE;
            }
            let val = unsafe { *argv };
            js_array_is_array(ctx, val)
        }

        fn object_create(
            ctx: *mut JSContext,
            _this_val: *mut JSValue,
            argc: i32,
            argv: *mut JSValue,
        ) -> JSValue {
            if argc < 1 {
                return JSValue::EXCEPTION;
            }
            let ctx = unsafe { &mut *(ctx as *mut JSContextImpl) };
            let proto = unsafe { *argv };
            js_object_create(ctx, proto)
        }

        fn object_define_property(
            ctx: *mut JSContext,
            _this_val: *mut JSValue,
            argc: i32,
            argv: *mut JSValue,
        ) -> JSValue {
            if argc < 3 {
                return JSValue::EXCEPTION;
            }
            let ctx = unsafe { &mut *(ctx as *mut JSContextImpl) };
            let obj = unsafe { *argv };
            let key = unsafe { *argv.add(1) };
            let desc = unsafe { *argv.add(2) };
            let val = js_get_property_str(ctx, desc, "value");
            js_object_define_property(ctx, obj, key, val)
        }

        fn array_push(
            ctx: *mut JSContext,
            this_val: *mut JSValue,
            argc: i32,
            argv: *mut JSValue,
        ) -> JSValue {
            let ctx = unsafe { &mut *(ctx as *mut JSContextImpl) };
            if argc < 1 {
                return js_array_push(ctx, unsafe { *this_val }, JSValue::UNDEFINED);
            }
            let val = unsafe { *argv };
            js_array_push(ctx, unsafe { *this_val }, val)
        }

        fn array_pop(
            ctx: *mut JSContext,
            this_val: *mut JSValue,
            _argc: i32,
            _argv: *mut JSValue,
        ) -> JSValue {
            let ctx = unsafe { &mut *(ctx as *mut JSContextImpl) };
            js_array_pop(ctx, unsafe { *this_val })
        }

        fn object_get_prototype_of(
            ctx: *mut JSContext,
            _this_val: *mut JSValue,
            argc: i32,
            argv: *mut JSValue,
        ) -> JSValue {
            if argc < 1 {
                return JSValue::EXCEPTION;
            }
            let ctx = unsafe { &mut *(ctx as *mut JSContextImpl) };
            let obj = unsafe { *argv };
            js_object_get_prototype_of(ctx, obj)
        }

        fn date_now(
            ctx: *mut JSContext,
            _this_val: *mut JSValue,
            _argc: i32,
            _argv: *mut JSValue,
        ) -> JSValue {
            let ctx = unsafe { &mut *(ctx as *mut JSContextImpl) };
            js_date_now(ctx)
        }

        let mut mem = vec![0u8; 4096];
        let mut ctx = JS_NewContext(&mut mem);
        let def_obj = JSCFunctionDef {
            func: JSCFunctionType { constructor: Some(object_ctor) },
            name: JSValue::UNDEFINED,
            def_type: JSCFunctionDefEnum::Constructor as u8,
            arg_count: 0,
            magic: 0,
        };
        let def_arr = JSCFunctionDef {
            func: JSCFunctionType { constructor: Some(array_ctor) },
            name: JSValue::UNDEFINED,
            def_type: JSCFunctionDefEnum::Constructor as u8,
            arg_count: 1,
            magic: 0,
        };
        let def_keys = JSCFunctionDef {
            func: JSCFunctionType { generic: Some(object_keys) },
            name: JSValue::UNDEFINED,
            def_type: JSCFunctionDefEnum::Generic as u8,
            arg_count: 1,
            magic: 0,
        };
        let def_is_array = JSCFunctionDef {
            func: JSCFunctionType { generic: Some(array_is_array) },
            name: JSValue::UNDEFINED,
            def_type: JSCFunctionDefEnum::Generic as u8,
            arg_count: 1,
            magic: 0,
        };
        let def_create = JSCFunctionDef {
            func: JSCFunctionType { generic: Some(object_create) },
            name: JSValue::UNDEFINED,
            def_type: JSCFunctionDefEnum::Generic as u8,
            arg_count: 1,
            magic: 0,
        };
        let def_define = JSCFunctionDef {
            func: JSCFunctionType { generic: Some(object_define_property) },
            name: JSValue::UNDEFINED,
            def_type: JSCFunctionDefEnum::Generic as u8,
            arg_count: 3,
            magic: 0,
        };
        let def_get_proto = JSCFunctionDef {
            func: JSCFunctionType { generic: Some(object_get_prototype_of) },
            name: JSValue::UNDEFINED,
            def_type: JSCFunctionDefEnum::Generic as u8,
            arg_count: 1,
            magic: 0,
        };
        fn math_abs(x: f64) -> f64 {
            x.abs()
        }
        fn math_floor(x: f64) -> f64 {
            x.floor()
        }
        let def_abs = JSCFunctionDef {
            func: JSCFunctionType { f_f: Some(math_abs) },
            name: JSValue::UNDEFINED,
            def_type: JSCFunctionDefEnum::FF as u8,
            arg_count: 1,
            magic: 0,
        };
        let def_floor = JSCFunctionDef {
            func: JSCFunctionType { f_f: Some(math_floor) },
            name: JSValue::UNDEFINED,
            def_type: JSCFunctionDefEnum::FF as u8,
            arg_count: 1,
            magic: 0,
        };
        let def_push = JSCFunctionDef {
            func: JSCFunctionType { generic: Some(array_push) },
            name: JSValue::UNDEFINED,
            def_type: JSCFunctionDefEnum::Generic as u8,
            arg_count: 1,
            magic: 0,
        };
        let def_pop = JSCFunctionDef {
            func: JSCFunctionType { generic: Some(array_pop) },
            name: JSValue::UNDEFINED,
            def_type: JSCFunctionDefEnum::Generic as u8,
            arg_count: 0,
            magic: 0,
        };
        let def_date_now = JSCFunctionDef {
            func: JSCFunctionType { generic: Some(date_now) },
            name: JSValue::UNDEFINED,
            def_type: JSCFunctionDefEnum::Generic as u8,
            arg_count: 0,
            magic: 0,
        };
        let table = [
            def_obj,
            def_arr,
            def_keys,
            def_is_array,
            def_create,
            def_abs,
            def_floor,
            def_define,
            def_push,
            def_pop,
            def_get_proto,
            def_date_now,
        ];
        JS_SetCFunctionTable(&mut ctx, &table);
        let _ = JS_RegisterStdlibMinimal(&mut ctx);
        let obj = eval_ret(&mut ctx, "Object()");
        assert_eq!(JS_GetClassID(&mut ctx, obj), JSObjectClassEnum::Object as i32);
        let arr = eval_ret(&mut ctx, "Array(2)");
        let len = JS_GetPropertyStr(&mut ctx, arr, "length");
        assert_eq!(JS_ToInt32(&mut ctx, len).unwrap(), 2);
        let arr2 = eval_ret(&mut ctx, "Array(1,2)");
        let v0 = JS_GetPropertyUint32(&mut ctx, arr2, 0);
        let v1 = JS_GetPropertyUint32(&mut ctx, arr2, 1);
        assert_eq!(JS_ToInt32(&mut ctx, v0).unwrap(), 1);
        assert_eq!(JS_ToInt32(&mut ctx, v1).unwrap(), 2);
        let keys = eval_ret(&mut ctx, "Object.keys({a:1})");
        let k0 = JS_GetPropertyUint32(&mut ctx, keys, 0);
        let mut buf = JSCStringBuf { buf: [0u8; 5] };
        let k0s = JS_ToString(&mut ctx, k0);
        let ks = JS_ToCString(&mut ctx, k0s, &mut buf);
        assert_eq!(ks, "a");
        let is_arr = eval_ret(&mut ctx, "Array.isArray([])");
        assert_eq!(is_arr, JSValue::TRUE);
        let created = eval_ret(&mut ctx, "Object.create({})");
        assert_eq!(JS_GetClassID(&mut ctx, created), JSObjectClassEnum::Object as i32);
        let abs_v = eval_ret(&mut ctx, "Math.abs(-3)");
        assert_eq!(JS_ToInt32(&mut ctx, abs_v).unwrap(), 3);
        let floor_v = eval_ret(&mut ctx, "Math.floor(1.9)");
        let fv = JS_ToNumber(&mut ctx, floor_v).unwrap();
        assert!((fv - 1.0).abs() < 1e-9);
        let _ = JS_Eval(&mut ctx, "o = {}", "test.js", 0);
        let _ = JS_Eval(&mut ctx, "Object.defineProperty(o, \"x\", {value: 7})", "test.js", 0);
        let ox = eval_ret(&mut ctx, "o.x");
        assert_eq!(JS_ToInt32(&mut ctx, ox).unwrap(), 7);
        let _ = JS_Eval(&mut ctx, "arr = []", "test.js", 0);
        let _ = JS_Eval(&mut ctx, "arr.push(1)", "test.js", 0);
        let pv = eval_ret(&mut ctx, "arr.pop()");
        assert_eq!(JS_ToInt32(&mut ctx, pv).unwrap(), 1);
        let keys_empty = eval_ret(&mut ctx, "Object.keys([])");
        let klen = JS_GetPropertyStr(&mut ctx, keys_empty, "length");
        assert_eq!(JS_ToInt32(&mut ctx, klen).unwrap(), 0);
        let proto = eval_ret(&mut ctx, "Object.getPrototypeOf({})");
        assert_eq!(JS_GetClassID(&mut ctx, proto), JSObjectClassEnum::Object as i32);
        let _ = JS_Eval(&mut ctx, "p = {a: 5}", "test.js", 0);
        let _ = JS_Eval(&mut ctx, "o = Object.create(p)", "test.js", 0);
        let oa = eval_ret(&mut ctx, "o.a");
        assert_eq!(JS_ToInt32(&mut ctx, oa).unwrap(), 5);
        let proto2 = eval_ret(&mut ctx, "Object.getPrototypeOf(o)");
        let pa = JS_GetPropertyStr(&mut ctx, proto2, "a");
        assert_eq!(JS_ToInt32(&mut ctx, pa).unwrap(), 5);
        let now = eval_ret(&mut ctx, "Date.now()");
        let n = JS_ToNumber(&mut ctx, now).unwrap();
        assert!(n > 0.0);
    }

    #[test]
    fn eval_basic_literals() {
        let mut mem = vec![0u8; 4096];
        let mut ctx = JS_NewContext(&mut mem);
        let v = eval_ret(&mut ctx, "42");
        let n = JS_ToInt32(&mut ctx, v).expect("int32");
        assert_eq!(n, 42);
        let t = eval_ret(&mut ctx, "true");
        assert_eq!(t, JSValue::TRUE);
        let s = eval_ret(&mut ctx, "\"hi\"");
        let mut buf = JSCStringBuf { buf: [0u8; 5] };
        let ss = JS_ToString(&mut ctx, s);
        let out = JS_ToCString(&mut ctx, ss, &mut buf);
        assert_eq!(out, "hi");
        let len = JS_GetPropertyStr(&mut ctx, s, "length");
        assert_eq!(JS_ToInt32(&mut ctx, len).unwrap(), 2);
        let e = eval_ret(&mut ctx, "1+2*3");
        let n = JS_ToNumber(&mut ctx, e).expect("number");
        assert!((n - 7.0).abs() < 1e-9);
        let e2 = eval_ret(&mut ctx, "(1+2)*3");
        let n2 = JS_ToNumber(&mut ctx, e2).expect("number");
        assert!((n2 - 9.0).abs() < 1e-9);
        let e3 = eval_ret(&mut ctx, "1.5+1");
        let n3 = JS_ToNumber(&mut ctx, e3).expect("number");
        assert!((n3 - 2.5).abs() < 1e-9);
        let arr = eval_ret(&mut ctx, "[1, 2]");
        let a1 = JS_GetPropertyUint32(&mut ctx, arr, 0);
        let a2 = JS_GetPropertyUint32(&mut ctx, arr, 1);
        assert_eq!(JS_ToInt32(&mut ctx, a1).unwrap(), 1);
        assert_eq!(JS_ToInt32(&mut ctx, a2).unwrap(), 2);
        let obj = eval_ret(&mut ctx, "{a: 3}");
        let oa = JS_GetPropertyStr(&mut ctx, obj, "a");
        assert_eq!(JS_ToInt32(&mut ctx, oa).unwrap(), 3);
        let nested = eval_ret(&mut ctx, "[1, [2, 3]]");
        let inner = JS_GetPropertyUint32(&mut ctx, nested, 1);
        let inner_val = JS_GetPropertyUint32(&mut ctx, inner, 0);
        assert_eq!(JS_ToInt32(&mut ctx, inner_val).unwrap(), 2);
        let expr = eval_ret(&mut ctx, "([1,2])[0]");
        assert_eq!(JS_ToInt32(&mut ctx, expr).unwrap(), 1);
        let _ = JS_Eval(&mut ctx, "x = 7", "test.js", 0);
        let xv = eval_ret(&mut ctx, "x");
        assert_eq!(JS_ToInt32(&mut ctx, xv).unwrap(), 7);
        let _ = JS_Eval(&mut ctx, "obj = {a: 1}", "test.js", 0);
        let _ = JS_Eval(&mut ctx, "obj.a = 4", "test.js", 0);
        let ov = eval_ret(&mut ctx, "obj.a");
        assert_eq!(JS_ToInt32(&mut ctx, ov).unwrap(), 4);
        let _ = JS_Eval(&mut ctx, "arr = []", "test.js", 0);
        let _ = JS_Eval(&mut ctx, "arr[\"0\"] = 9", "test.js", 0);
        let av = eval_ret(&mut ctx, "arr[0]");
        assert_eq!(JS_ToInt32(&mut ctx, av).unwrap(), 9);
    }

    #[test]
    fn eval_string_concat() {
        let mut mem = vec![0u8; 4096];
        let mut ctx = JS_NewContext(&mut mem);
        let v = eval_ret(&mut ctx, "\"a\" + \"b\"");
        let mut buf = JSCStringBuf { buf: [0u8; 5] };
        let vs = JS_ToString(&mut ctx, v);
        let s = JS_ToCString(&mut ctx, vs, &mut buf);
        assert_eq!(s, "ab");
        let _ = JS_Eval(&mut ctx, "o = {}", "test.js", 0);
        let ov = eval_ret(&mut ctx, "o + 1");
        let ovs = JS_ToString(&mut ctx, ov);
        let os = JS_ToCString(&mut ctx, ovs, &mut buf);
        assert_eq!(os, "[object Object]1");
    }

    #[test]
    fn eval_default_returns_undefined() {
        let mut mem = vec![0u8; 4096];
        let mut ctx = JS_NewContext(&mut mem);
        let v = JS_Eval(&mut ctx, "1+1", "test.js", 0);
        assert_eq!(v, JSValue::UNDEFINED);
    }

    #[test]
    fn eval_semicolon_sequence() {
        let mut mem = vec![0u8; 4096];
        let mut ctx = JS_NewContext(&mut mem);
        let v = eval_ret(&mut ctx, "x = 1; x + 2");
        assert_eq!(JS_ToInt32(&mut ctx, v).unwrap(), 3);
        let v2 = JS_Eval(&mut ctx, "y = 2; y + 1", "test.js", 0);
        assert_eq!(v2, JSValue::UNDEFINED);
        let yv = eval_ret(&mut ctx, "y");
        assert_eq!(JS_ToInt32(&mut ctx, yv).unwrap(), 2);
    }

    #[test]
    fn parse_and_run() {
        let mut mem = vec![0u8; 4096];
        let mut ctx = JS_NewContext(&mut mem);
        let code = JS_Parse(&mut ctx, "42", "test.js", 0);
        let res = JS_Run(&mut ctx, code);
        let n = JS_ToInt32(&mut ctx, res).expect("int32");
        assert_eq!(n, 42);
    }

    #[test]
    fn eval_json_flag() {
        let mut mem = vec![0u8; 4096];
        let mut ctx = JS_NewContext(&mut mem);
        let v = JS_Eval(&mut ctx, "{\"a\": [1, true, null]}", "test.js", JS_EVAL_JSON);
        let arr = JS_GetPropertyStr(&mut ctx, v, "a");
        let v0 = JS_GetPropertyUint32(&mut ctx, arr, 0);
        let v1 = JS_GetPropertyUint32(&mut ctx, arr, 1);
        let v2 = JS_GetPropertyUint32(&mut ctx, arr, 2);
        assert_eq!(JS_ToInt32(&mut ctx, v0).unwrap(), 1);
        assert_eq!(v1, JSValue::TRUE);
        assert_eq!(v2, JSValue::NULL);
        let parsed = JS_Parse(&mut ctx, "{\"x\": 2}", "test.js", JS_EVAL_JSON);
        let ran = JS_Run(&mut ctx, parsed);
        let xv = JS_GetPropertyStr(&mut ctx, ran, "x");
        assert_eq!(JS_ToInt32(&mut ctx, xv).unwrap(), 2);
    }

    #[test]
    fn throw_sets_exception() {
        let mut mem = vec![0u8; 4096];
        let mut ctx = JS_NewContext(&mut mem);
        let _ = JS_ThrowError(&mut ctx, JSObjectClassEnum::TypeError, "boom");
        let ex = JS_GetException(&mut ctx);
        let mut buf = JSCStringBuf { buf: [0u8; 5] };
        let es = JS_ToString(&mut ctx, ex);
        let s = JS_ToCString(&mut ctx, es, &mut buf);
        assert_eq!(s, "boom");
    }

    #[test]
    fn numeric_property_names_on_arrays() {
        let mut mem = vec![0u8; 4096];
        let mut ctx = JS_NewContext(&mut mem);
        let arr = JS_NewArray(&mut ctx, 0);
        let v = JS_NewInt32(&mut ctx, 3);
        let ok = JS_SetPropertyStr(&mut ctx, arr, "0", v);
        assert!(!ok.is_exception());
        let got = JS_GetPropertyUint32(&mut ctx, arr, 0);
        let n = JS_ToInt32(&mut ctx, got).expect("int32");
        assert_eq!(n, 3);
        let bad = JS_SetPropertyStr(&mut ctx, arr, "2", v);
        assert!(bad.is_exception());
    }

    #[test]
    fn object_to_string_defaults() {
        let mut mem = vec![0u8; 4096];
        let mut ctx = JS_NewContext(&mut mem);
        let obj = JS_NewObject(&mut ctx);
        let arr = JS_NewArray(&mut ctx, 0);
        let mut buf = JSCStringBuf { buf: [0u8; 5] };
        let os_val = JS_ToString(&mut ctx, obj);
        let os = JS_ToCString(&mut ctx, os_val, &mut buf);
        assert_eq!(os, "[object Object]");
        let as_val = JS_ToString(&mut ctx, arr);
        let as_ = JS_ToCString(&mut ctx, as_val, &mut buf);
        assert_eq!(as_, "[object Array]");
    }
}
