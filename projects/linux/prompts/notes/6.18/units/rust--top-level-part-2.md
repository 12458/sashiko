# RUST — (top level) (part 2)

This unit comprises the kernel's procedural macros (`rust/macros/*`, implementing `module!`, `#[vtable]`, `#[export]`, `concat_idents!`, `paste!`, and `#[kunit_tests]`), the UAPI bindgen wrapper crate (`rust/uapi/*`), and the host build script `scripts/generate_rust_target.rs` that generates custom `rustc` target JSON specifications from Kconfig (`auto.conf`).

## Architecture & Data Flow

- In `module!`, `THIS_MODULE` wraps `__this_module.get()` for loadable modules (`#[cfg(MODULE)]`) but wraps `::core::ptr::null_mut()` when built-in (`#[cfg(not(MODULE))]`), so consumers of `THIS_MODULE` (`ThisModule::as_ptr`) must tolerate a null `bindings::module` pointer in built-in builds. Broken by: dereferencing `THIS_MODULE.as_ptr()` without a null check or `#[cfg(MODULE)]` guard in built-in-reachable code. [F156]
- `module!` (`rust/macros/module.rs`) must keep `static mut __MOD: MaybeUninit<{type_}>`, `unsafe fn __init`, `unsafe fn __exit`, and the `#[no_mangle] pub extern "C"` lifecycle functions (`init_module`, `cleanup_module`, `__{ident}_init`, `__{ident}_exit`) inside double-nested private modules (`mod __module_init { mod __module_init { ... } }`) so safe Rust code in the invoking module cannot call them. [F182]
- In `rust/macros/vtable.rs`, the `is_trait` loop in `vtable` must insert each emitted `gen_const_name` (`HAS_*`) into `consts` so duplicate method names in `functions` (such as `#[cfg]`-conditional method overloads) are skipped on later iterations instead of emitting duplicate `const HAS_*` items. [F171]
- In `paste::expand`, when unwrapping `Delimiter::None` groups around `::` at `tokens[i + 1..=i + 2]`, `tokens.splice(i + 3..i + 4, ...)` must execute before `tokens.splice(i..i + 1, ...)` in a reverse loop (`.rev()`) so multi-token splices at `i` do not shift index `i + 3` or unvisited indices. [F161]
- In `rust/macros/quote.rs`, the `quote_spanned!` rules for `#$id:ident` and `#(#$id:ident)*` must be ordered before the literal `#` punctuation rule (`@proc $v:ident $span:ident # $($tt:tt)*`), because `# $($tt:tt)*` matches any token stream starting with `#`. [F173]

## Object Lifetimes

- In `module!` (`rust/macros/module.rs`), `init_module` is placed in `#[link_section = ".init.text"]` and must never be retained or called after module initialization completes because `.init.text` may be freed afterward. [F186]

## API Contracts

- In a `#[vtable]` trait, the default body of every optional method must call `build_error!(VTABLE_DEFAULT_ERROR)` rather than returning a runtime fallback such as `Err(EINVAL)` or `Ok(...)`, because C vtables install `NULL` when `HAS_*` is `false` and the default Rust method is never meant to execute. [F157]
- `ModuleInfo::parse` (`rust/macros/module.rs`) requires `module!` keys to include `REQUIRED_KEYS` (`type`, `name`, `license`), appear at most once, end with a comma, and follow the exact `EXPECTED_KEYS` relative order `["type", "name", "authors", "description", "license", "alias", "firmware"]`, with ASCII-only string literals for all string fields except `description`. [F158]
- Callers of `KernelConfig::has` in `scripts/generate_rust_target.rs` must pass the Kconfig symbol name without the `CONFIG_` prefix, because `KernelConfig::has` unconditionally prepends `"CONFIG_"` before querying `self.0` and relies on omitting the prefix to avoid `fixdep` tracking. [F164]

## External Contracts

- In `rust/macros/export.rs`, `#[export]` must emit the `const _: () = { if true { ::kernel::bindings::#name } else { #name }; };` type-unification check alongside `#[no_mangle]`, and any `#[export]` function must have its C declaration in `rust/bindings/bindings_helper.h`. Broken by: exporting a Rust function to C with raw `#[no_mangle]` or weakening the unification check in `export()`. [F184]
- `ModInfoBuilder::emit_base` (`rust/macros/module.rs`) must format NUL-terminated `.modinfo` strings with a `{module}.` prefix (`"{module}.{field}={content}\0"`) for built-in modules (`#[cfg(not(MODULE))]`) and without a prefix (`"{field}={content}\0"`) for loadable modules (`#[cfg(MODULE)]`). [F188]
- In `module!` (`rust/macros/module.rs`), loadable modules must place `init_module` in `.init.text` (`__UNIQUE_ID___addressable_init_module` in `.init.data`) and `cleanup_module` in `.exit.text` (`__UNIQUE_ID___addressable_cleanup_module` in `.exit.data`) with `#[used(compiler)]` addressable statics; built-in modules (`#[cfg(not(MODULE))]`) must place `__{ident}_initcall` in `.initcall6.init` as a `global_asm!` relative offset (`.long __{ident}_init - .`) when `CONFIG_HAVE_ARCH_PREL32_RELOCATIONS` is set, and as a static `extern "C" fn() -> c_int` pointer otherwise. [F189, F191]
- `rust/uapi/lib.rs` must keep `pin_init::MaybeZeroable` in scope and manually define `__kernel_size_t`, `__kernel_ssize_t`, and `__kernel_ptrdiff_t` before `include!(concat!(env!("OBJTREE"), "/rust/uapi/uapi_generated.rs"))` because `rust/bindgen_parameters` blocklists those three C types and derives `MaybeZeroable` on all UAPI structs and unions. Broken by: adding `--blocklist-type` or `--with-derive-custom-*` in `rust/bindgen_parameters` without importing the corresponding type or trait in `rust/uapi/lib.rs`. [F200]
- In `scripts/generate_rust_target.rs`, `target-pointer-width` must be serialized as a JSON integer (`64` or `32`) when `cfg.rustc_version_atleast(1, 91, 0)` is true, and as a JSON string (`"64"` or `"32"`) on earlier `rustc` versions. [F168]
- In `scripts/generate_rust_target.rs`, endianness is determined by checking `cfg.has("CPU_BIG_ENDIAN")` (emitting `"target-endian": "big"`) rather than `CPU_LITTLE_ENDIAN`, because little-endian architectures such as x86 may not declare `CONFIG_CPU_LITTLE_ENDIAN` and rely on `rustc`'s little-endian default. [F170]
- In `scripts/generate_rust_target.rs`, `X86_32` target generation is only supported when `cfg.has("UML")` is true and must panic otherwise, because non-UML 32-bit x86 kernels require `regparm` ABI support not available in `rustc`. [F194]
- On `X86_64` in `scripts/generate_rust_target.rs`, `features` must include `+retpoline-external-thunk,+retpoline-indirect-branches,+retpoline-indirect-calls` when `MITIGATION_RETPOLINE` is set and `+harden-sls-ijmp,+harden-sls-ret` when `MITIGATION_SLS` is set, matching Clang's `-mretpoline-external-thunk` and `-mharden-sls=all` expansions. [F195]

## Known Pitfalls

- `helpers::try_string` (used by `expect_string`, `expect_string_ascii`, and `expect_string_array` in `module!`) panics on raw string literals (`r"..."`) and any string literal containing a backslash (`\`), such as escape sequences `\n` or `\"`. [F159]
- `#[vtable]` (`rust/macros/vtable.rs`) only scans direct `fn <ident>` tokens in the outer brace group of a `trait` or `impl` block when generating `HAS_<UPPERCASE_FN>` constants, so methods generated by inner macros inside a `#[vtable]` block must explicitly declare their `const HAS_<NAME>: bool` items. [F199]
- In `scripts/generate_rust_target.rs`, `KernelConfig::from_stdin` must call `line.clear()` at the top of the read loop before `handle.read_line(&mut line)` so that `continue` on comment lines (`line.starts_with('#')`) does not leave comment text in `line` for the next `read_line` to append onto. [F175]
- `Value::fmt` and `TargetSpec::fmt` in `scripts/generate_rust_target.rs` interpolate keys and `Value::String` payloads directly inside `"{}"` without JSON escaping, so strings pushed into `TargetSpec` must not contain double quotes, backslashes, or control characters unless JSON string escaping is added to `Value::fmt`. [F179]
