# RUST — (top level) (part 2)

This unit covers kernel procedural macros (`rust/macros/*` implementing `module!`, `#[vtable]`, `#[export]`, `concat_idents!`, `paste!`, and `#[kunit_tests]`), the UAPI bindgen wrapper crate (`rust/uapi/*`), and the host build script `scripts/generate_rust_target.rs` that generates custom `rustc` target JSON specifications from Kconfig (`auto.conf`).

## Architecture & Data Flow

- In `paste::expand`, when unwrapping `Delimiter::None` groups around `::` at `tokens[i + 1..=i + 2]`, the loop must iterate in reverse (`.rev()`) and execute `tokens.splice(i + 3..i + 4, ...)` before `tokens.splice(i..i + 1, ...)` so multi-token splices at `i` do not shift index `i + 3` or unvisited indices. [F161]

## API Contracts

- Callers of `KernelConfig::has` in `scripts/generate_rust_target.rs` must pass Kconfig symbol names without the `CONFIG_` prefix, because `KernelConfig::has` unconditionally prepends `"CONFIG_"` before querying `self.0` and relies on omitting the prefix to avoid `fixdep` tracking. [F164]

## External Contracts

- `rust/uapi/lib.rs` must keep `pin_init::MaybeZeroable` in scope and manually define `__kernel_size_t`, `__kernel_ssize_t`, and `__kernel_ptrdiff_t` before `include!(concat!(env!("OBJTREE"), "/rust/uapi/uapi_generated.rs"))`, because `rust/bindgen_parameters` blocklists those three C types and derives `MaybeZeroable` on all UAPI structs and unions. Broken by: adding `--blocklist-type` or `--with-derive-custom-*` entries in `rust/bindgen_parameters` without providing matching definitions or imports in `rust/uapi/lib.rs`. [F200]
- In `scripts/generate_rust_target.rs`, `target-pointer-width` must be serialized as a JSON integer (`64` or `32`) when `cfg.rustc_version_atleast(1, 91, 0)` is true, and as a JSON string (`"64"` or `"32"`) on earlier `rustc` versions. [F168]
- In `scripts/generate_rust_target.rs`, `X86_32` target generation is only supported when `cfg.has("UML")` is true and must panic otherwise, because non-UML 32-bit x86 kernels require `regparm` calling-convention ABI support not available in `rustc`. [F194]
- On `X86_64` in `scripts/generate_rust_target.rs`, `features` must include `+retpoline-external-thunk,+retpoline-indirect-branches,+retpoline-indirect-calls` when `MITIGATION_RETPOLINE` is set and `+harden-sls-ijmp,+harden-sls-ret` when `MITIGATION_SLS` is set, matching Clang's `-mretpoline-external-thunk` and `-mharden-sls=all` expansions. [F195]
- In `scripts/generate_rust_target.rs`, endianness must be determined by checking `cfg.has("CPU_BIG_ENDIAN")` (emitting `"target-endian": "big"`) rather than `CPU_LITTLE_ENDIAN`, because little-endian architectures such as x86 may not declare `CONFIG_CPU_LITTLE_ENDIAN` and rely on `rustc`'s little-endian default. [F170]

## Known Pitfalls

- `Value::fmt` and `TargetSpec::fmt` in `scripts/generate_rust_target.rs` interpolate keys and `Value::String` payloads directly inside `"{}"` without JSON escaping, so strings pushed into `TargetSpec` must not contain double quotes, backslashes, or raw newlines unless JSON string escaping is added to `Value::fmt`. [F179]
- In `scripts/generate_rust_target.rs`, `KernelConfig::from_stdin` must call `line.clear()` at the top of the read loop before `handle.read_line(&mut line)` so that `continue` on comment lines (`line.starts_with('#')`) does not leave comment text in `line` for `read_line` to append onto. [F175]
