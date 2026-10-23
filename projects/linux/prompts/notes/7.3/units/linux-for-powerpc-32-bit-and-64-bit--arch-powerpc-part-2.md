# LINUX FOR POWERPC (32-BIT AND 64-BIT) — arch/powerpc (part 2)

Provides PowerPC internal (`asm/`) and UAPI headers plus processor cache sysfs (`cacheinfo.c`), alignment emulation (`align.c`), early framebuffer text (`btext.c`), and CPU feature/setup tables (`cpu_specs_*.h`, `cpu_setup_power.c`).

## Architecture & Data Flow

- In `cpu_setup_power.c`, every SPR initialization added to `__setup_cpu_power{7,8,9,10}()` must be mirrored in `__restore_cpu_power{7,8,9,10}()`. [F36]
- In `asm/switch_to.h`, `clear_task_ebb()` on `CONFIG_PPC_BOOK3S_64` must zero all coupled EBB and PMU thread fields (`ebbrr`, `ebbhr`, `bescr`, `mmcr2`, `mmcr0`, `siar`, `sdar`, `sier`, `used_ebb`) together. [F41]
- In `cacheinfo.c`, `cache_associativity()` must verify `nr_sets`, `size`, and `line_size` are strictly positive (after handling `nr_sets == 1`) before computing `(size / nr_sets) / line_size`, returning `-ENODEV` if any property is zero or missing. [F51]
- In `cacheinfo.c`, `cache_dir::kobj` is a standalone `struct kobject *` while `cache_index_dir::kobj` is embedded; `kobj_to_cache_index_dir()` and `index_kobj_to_cache()` must only be called on `cache_index_type` kobjects, and every attribute on `cache_index_type` (`cache_index_default_attrs`, `cache_index_opt_attrs`) must be embedded in `struct kobj_attribute` for `cache_index_show()`. [F93, F94]
- In `asm/thread_info.h`, `MIN_THREAD_SHIFT` is `CONFIG_THREAD_SHIFT + 1` when `CONFIG_KASAN && CONFIG_THREAD_SHIFT < 15`, and `THREAD_ALIGN_SHIFT` is `THREAD_SHIFT + 1` under `CONFIG_VMAP_STACK` so `sp & (1 << THREAD_SHIFT)` detects overflow. In `arch_within_stack_frames()`, `[obj, obj + len)` is `GOOD_FRAME` only if entirely between `prev_frame + STACK_FRAME_PARAMS` and `frame` within `[stack, stackend)`. [F54, F115]
- In `asm/xics.h`, per-CPU `xics_cppr` (`stack[MAX_NUM_PRIORITIES]`) reserves `index == 0` for `xics_set_base_cppr()`, and `xics_push_cppr()` / `xics_pop_cppr()` must check `index >= MAX_NUM_PRIORITIES - 1` and `index < 1` before pre-incrementing or pre-decrementing `index`. [F56]
- In `cpu_specs_*.h`, `cpu_specs[]` entries with a more specific `pvr_mask` must precede broader family entries, with the catch-all (`pvr_mask = 0`) last, because `identify_cpu()` returns on the first match. [F67]
- In `asm/sstep.h`, `enum instruction_type` values from `LOAD` through `STCX` must remain contiguous, and `OP_IS_LOAD()`, `OP_IS_STORE()`, and `OP_IS_LOAD_STORE()` must receive `GETTYPE(op.type)` rather than raw `op.type` (which has flags/size above `INSTR_TYPE_MASK`). [F68]
- In `unsafe_copy_from_user()` and `unsafe_copy_to_user()` (`asm/uaccess.h`), byte index `_i` advanced by the 8-byte loop must be incremented by 4 in the `(_len & 4)` block and by 2 in the `(_len & 2)` block. [F76]
- In `asm/text-patching.h`, `R2_STACK_OFFSET` (24 on ABIv2, 40 otherwise) used by `PPC_INST_LD_TOC` must match `STK_GOT` in `asm/ppc_asm.h`, and `ppc_function_entry()` must skip the 2-instruction TOC prologue on ABIv2 or dereference `struct func_desc` on ABIv1. Broken by: patching at `ppc_global_function_entry()`. [F117]

## Synchronization

- In `cacheinfo.c`, `cacheinfo_cpu_online()`, `cacheinfo_cpu_offline()`, `cacheinfo_teardown()`, and `cacheinfo_rebuild()` must be called in process context holding the CPU hotplug lock. `cacheinfo_cpu_online()` must populate the cache chain and `shared_cpu_map` via `cache_chain_instantiate()` before calling `cacheinfo_sysfs_populate()`, and `cacheinfo_create_index_dir()` must assign `index_dir->cache` before `kobject_init_and_add()`. [F9, F10, F121]
- In `asm/simple_spinlock.h` on `CONFIG_PPC64`, `__arch_spin_trylock()` and `__arch_write_trylock()` must store `LOCK_TOKEN` (`0x80000000 | smp_processor_id()`) for `splpar_spin_yield()` / `splpar_rw_yield()`, and `__arch_read_trylock()` must sign-extend the `lwarx` result (`__DO_SIGN_EXTEND` / `extsw`) before `addic.` and `ble-`. Wait loops (`arch_spin_lock()`, `arch_read_lock()`, `arch_write_lock()`) must restore `HMT_medium()` after `HMT_low()` and only call `splpar_*_yield()` when `is_shared_processor()` is true. [F13, F81, F112]
- `vas_user_win_add_mm_context()` must issue `PPC_CP_ABORT` after `mm_context_add_vas_window()` to clear any pending unpaired COPY, and all accesses to `struct vas_user_win_ref.vma` across `mmap()` and DLPAR close/reopen paths must hold `vas_user_win_ref.mmap_mutex`. [F16, F17]
- On 32-bit builds (`!__is_defined(__powerpc64__)`), `get_tb()` must read `mftbu()`, `mftb()`, and `mftbu()` in a loop until upper words match, and `set_tb()` must zero `SPRN_TBWL` before writing `SPRN_TBWU` and `SPRN_TBWL`. [F18]

## Object Lifetimes

- `cache_init()` calls `of_node_get()` and links `cache->list` into `cache_list`, so `struct cache` from `new_cache()` must be freed via `release_cache()` (when `shared_cpu_map` becomes empty) rather than `kfree()`. CPU online/offline must update `shared_cpu_map` across the entire `cache->next_local` chain, and `cache_cpu_clear()` must save `next = cache->next_local` before `release_cache(cache)` while walking lower to higher levels. [F1, F31, F72]
- Once `cacheinfo_create_index_dir()` calls `kobject_init_and_add()`, `cache_index_dir` is owned by `cache_index_release()`, so failure must clean up via `kobject_put(&index_dir->kobj)` rather than `kfree(index_dir)` without linking `index_dir`. [F4]
- `cacheinfo_cpu_offline()` must call `remove_cache_dir()` and reset `per_cpu(cache_dir_pcpu, cpu_id)` to `NULL` before `cache_cpu_clear()`, and `remove_index_dirs()` must read `next = index->next` before `kobject_put(&index->kobj)`. [F5, F73]
- In `btext_find_display()` (`btext.c`), breaking out of `for_each_node_by_type(np, "display")` when `btext_initialize(np)` returns 0 requires `of_node_put(np)` before `break`. [F74]

## API Contracts

- `cache_lookup_or_instantiate()` and `cache_lookup_by_node_group()` borrow the caller's `device_node` without consuming its reference, so callers (`do_subsidiary_caches()`, `cache_chain_instantiate()`, `cache_lookup_by_cpu()`) must call `of_node_put()` on all return/break paths. [F3]
- Callers of `smu_queue_cmd()` must initialize all public fields of `struct smu_cmd`, and callers of `smu_queue_i2c()` must initialize `cmd->info` (including `info.data` for writes), `cmd->done`, and `cmd->misc` before queuing, without modifying them while `status == 1`. [F20]
- `__get_user_atomic_128_aligned(kaddr, uaddr, err)` requires `enable_kernel_vmx()` and `err` pre-initialized to `0` (`"0" (err)` is only modified in `.fixup`). `get_user(x, ptr)` and `__get_user_size_allowed(x, ptr, size, retval)` must zero `x` on `access_ok()` failure or load fault (including both words of a 64-bit load on PPC32) alongside returning `-EFAULT`. [F24, F25]
- `init_LPCR_ISA300()` and `init_LPCR_ISA206()` expect `lpes` as an unshifted 2-bit value because `init_LPCR_ISA300()` shifts `lpes` left by `LPCR_LPES_SH`. [F29]
- In `asm/word-at-a-time.h`, all `has_zero()` variants must return `unsigned long` (not `bool`) and store the byte mask into `*data`/`*bits`. On 64-bit LE, `zero_bytemask(mask)` computes `(1UL << mask) - 1` and requires `mask < 64` (at least one zero byte present). [F45, F97]
- In `asm/text-patching.h`, `create_branch()` must check `is_offset_in_branch_range(offset)` (`[-0x2000000, 0x1fffffc]` and 4-byte aligned) and mask `(flags & 0x3)` and `(offset & 0x03FFFFFC)`, returning `1` (`*instr = ppc_inst(0)`) if out of range. `patch_uint()`/`patch_u32()` and `patch_ulong()`/`patch_u64()` require `addr` aligned to `sizeof(unsigned int)` and `sizeof(unsigned long)`, returning `-EINVAL` if unaligned. [F55, F91]
- In `asm/syscall.h`, `syscall_get_arguments()` and `syscall_set_arguments()` must read/write `regs->orig_gpr3` (and `regs->gpr[3]` on set) for arg 0, `syscall_get_arguments()` must mask args with `0xffffffffUL` when `is_tsk_32bit_task(task)`, and `syscall_get_nr()` must return `int` so 32-bit `0xffffffff` in `gpr[0]` sign-extends to `-1`. [F59, F133]
- In `asm/xive-regs.h`, XIVE ESB MMIO offsets `XIVE_ESB_LOAD_EOI`, `XIVE_ESB_GET`, and `XIVE_ESB_SET_PQ_*` must be accessed with 8-byte loads. [F65]
- In `asm/sfp-machine.h`, `udiv_qrnnd(q, r, n1, n0, d)` requires `n1 < d` and MSB of `d` set (`UDIV_NEEDS_NORMALIZATION`). [F83]
- On `CONFIG_PPC64` without `CONFIG_KASAN`, `__memset16/32/64()` take a byte count `n`; callers with an element count must use `memset16/32/64()`, which scale `n` by 2, 4, and 8. [F87]
- On PowerNV, a VAS instance requires an open receive window (`vas_rx_win_open()`) for `@cop` before `vas_tx_win_open()`, and all attached send windows must close before `vas_win_close()` closes the receive window. [F105]
- In `asm/stackprotector.h`, `boot_init_stack_canary()` must be `__always_inline`, called only from non-returning functions, and on `CONFIG_PPC64` update both `current->stack_canary` and `get_paca()->canary`. [F123]
- Under `CONFIG_HOTPLUG_SMT`, `topology_is_core_online()` must return `true` if any sibling in `[cpu_first_thread_sibling(cpu), +threads_per_core)` is `cpu_online()`. Under `CONFIG_NUMA`, `early_cpu_to_node()` must return `0` when `numa_cpu_lookup_table[cpu] < 0`. [F125, F126]
- A `struct vio_driver` for a DMA-capable VDEVICE must implement `.get_desired_dma()`, or VIO probing under CMO fails with `-EINVAL`. On PPC64 with `CONFIG_HAVE_ARCH_HUGE_VMAP`, `arch_vmap_pud_supported()` and `arch_vmap_pmd_supported()` must check `radix_enabled()`. [F128, F137]

## Error Handling

- `fix_alignment()` and `emulate_spe()` return `1` on emulated instruction, `-EFAULT` on user fault, and `0` or negative errno otherwise; `fix_alignment()` must convert `0` returns from `emulate_dcbz()` and `emulate_loadstore()` into `1`. Unemulatable instructions (ISA 3.0 copy/paste, LARX/STCX) must return `-EIO` or `-EINVAL` without `WARN`, and `WARN_ON_ONCE(!user_mode(regs))` is only for `dcbz`. [F21, F70, F140]
- Optional cache sysfs `.show` callbacks (`size_show`, `line_size_show`, `nr_sets_show`, `associativity_show`) must return `-ENODEV` when a DT property is missing/invalid, and `cacheinfo_create_index_opt_attrs()` must probe each `attr->show()` into a `PAGE_SIZE` buffer and only call `sysfs_create_file()` when `rc > 0`. [F27, F64]

## External Contracts

- `syscall_get_error()` and `syscall_set_return_value()` must branch on `trap_is_scv(regs)`: `scv` encodes errors as negative `IS_ERR_VALUE` in `regs->gpr[3]`, while non-`scv` sets `0x10000000UL` in `regs->ccr` with a positive error in `regs->gpr[3]`. [F26]
- In `uapi/asm/ptrace.h`, `struct user_pt_regs` / `pt_regs` layout must match `PT_*` word indices (`PT_R0`..`PT_RESULT`) and have a 16-byte-multiple size. In `elf_vrregset_t` and 64-bit `struct sigcontext`, index 32 stores `vscr` at byte offset 12, 64-bit index 33 stores `vrsave` at offset 0, and `sigcontext.v_regs` must be `NULL` or 16-byte-aligned inside `vmx_reserve`. [F60, F61]
- In `asm/syscall_wrapper.h`, `__SYSCALL_DEFINEx`, `SYSCALL_DEFINE0`, and `COND_SYSCALL` must use the `sys_` prefix (not `__powerpc_sys_*`). On `CONFIG_PPC32` / `CONFIG_COMPAT` in `asm/syscalls.h`, wrappers with 64-bit args must split them into two 32-bit params with a dummy 32-bit pad whenever a 64-bit arg would start in an even GPR (`r4`, `r6`, `r8`), combining via `merge_64()`. [F62, F113]
- In `cpu_specs_e500mc.h`, `COMMON_USER_BOOKE` must include `PPC_FEATURE_BOOKE` on both `CONFIG_PPC64` and `CONFIG_PPC32`. UAPI ioctl binary payload arrays (`papr_sysparm_io_block::data`, `papr_phy_attest_io_block::payload`) must use `__u8[]` rather than `char[]`. [F63, F127]
- `vdso_arch_data` (`asm/vdso/arch_data.h`) defines `syscall_map` and `compat_syscall_map` as `__u32` arrays, so kernel code must not pass them to `unsigned long *` helpers (`set_bit()` / `bitmap_*()`). In `asm/vdso.h`, `V_FUNCTION_BEGIN(name)` must emit `.type name,@function`. [F98, F119]

## Known Pitfalls

- In `asm/syscall.h`, helpers taking an explicit `struct task_struct *task` (`syscall_get_arguments()`, `syscall_get_arch()`) must call `is_tsk_32bit_task(task)` rather than `is_32bit_task()`. [F66]
- In `cacheinfo.c`, `cache_do_one_devnode_split()` allocates `dcache` before `icache` (`dcache->next_local = icache`) and `cache_init()` prepends each to `cache_list`, placing `icache` first; `cache_lookup_by_node_group()` must call `cache_find_first_sibling()` to return `dcache`. [F71]
- In `btext.c`, `btext_setup_display()` and `btext_initialize()` must normalize depth 15 to `dispDeviceDepth = 16` (and fallback pitch `width * ((depth + 7) / 8)`) for `dispDeviceDepth >> 3`. `dispDeviceRowBytes` (`rb`) is a byte stride, so advancing `unsigned int *base` requires `dispDeviceRowBytes >> 2` or casting `base` to `char *` before adding `rb`. [F85, F89]
- In `asm/text-patching.h`, `patch_site_addr(s32 *site)` must cast `site` to `unsigned long` before adding `*site`. In `ppc_kallsyms_lookup_name()` on `CONFIG_PPC64_ELF_ABI_V1`, `dot_name` is `1 + KSYM_NAME_LEN` and must check `strnlen(name, KSYM_NAME_LEN) >= KSYM_NAME_LEN` before prepending `'.'`. [F90, F96]
- In `cpu_setup_power.c`, `__setup_cpu_power*()` and `__restore_cpu_power*()` must verify `MSR_HV` (via `init_hvmode_206(t)` or `mfmsr() & MSR_HV`) before accessing HV-privileged SPRs (`LPID`, `AMOR`, `PCR`, `LPCR`, `PSSCR`, `HFSCR`, `MMCRC`, `MMCRH`), and `SPRN_PTCR` writes must use `set_ptcr_when_no_uv()` or guard against `FW_FEATURE_ULTRAVISOR`. [F102, F107]
- In 64-bit non-prefixed inline asm in `asm/uaccess.h` (`__put_user_asm2_goto`, `__get_user_asm2_goto`), DS-form `std`/`ld` must use `DS_FORM_CONSTRAINT` (`"YZ<>"` / `"Z<>"`) rather than `"m<>"`. In `asm/simple_spinlock.h`, 4-operand `lwarx` must pass `IS_ENABLED(CONFIG_PPC64)` as the `eh` hint rather than `1`. [F108, F111]
- In `asm/vdso/timebase.h` and `asm/vdso/gettimeofday.h`, code shared with the 32-bit compat vDSO must guard 64-bit paths with `__powerpc64__` rather than `CONFIG_PPC64`. [F120]
