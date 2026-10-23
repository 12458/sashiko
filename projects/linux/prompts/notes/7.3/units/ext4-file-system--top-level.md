# EXT4 FILE SYSTEM — (top level)

This component contains the `user.*` extended attribute handler (`fs/ext4/xattr_user.c`), the ftrace event definitions for the ext4 subsystem (`include/trace/events/ext4.h`), and the userspace API header defining ext4-specific ioctl commands, flags, and structures (`include/uapi/linux/ext4.h`). Together they define the UAPI contract for ext4 ioctls, the user xattr VFS bridge, and the kernel tracepoint instrumentation schema.

## Architecture & Data Flow
- In `include/trace/events/ext4.h`, `TP_fast_assign` blocks reading on-disk `struct ext4_extent *` arguments must decode `ee_block` with `le32_to_cpu()`, physical start blocks with `ext4_ext_pblock()`, and extent lengths with `ext4_ext_get_actual_len()` rather than `le16_to_cpu(ex->ee_len)` [F428].
- When adding or removing an `EXT4_FC_REASON_*` enum constant before `EXT4_FC_REASON_MAX`, update `TRACE_DEFINE_ENUM()`, `show_fc_reason()`, and the hardcoded `%s:%u` format specifiers and `FC_REASON_NAME_STAT()` arguments in `TRACE_EVENT(ext4_fc_stats)` in lockstep [F419].
- `TRACE_EVENT(ext4_fc_cleanup)` casts `journal->j_private` directly to `struct super_block *` and dereferences `sb->s_dev`; callers must only invoke `trace_ext4_fc_cleanup()` while `journal->j_private` points to a valid `struct super_block` [F442].

## Synchronization
- `ext4__es_extent` (`ext4_es_insert_extent`, `ext4_es_cache_extent`), `ext4_es_find_extent_range_exit`, `ext4_es_lookup_extent_exit`, and `ext4_es_insert_delayed_extent` dereference `es` in `TP_fast_assign` outside `EXT4_I(inode)->i_es_lock`; callers must pass a caller-private `struct extent_status` snapshot rather than an in-tree node from `EXT4_I(inode)->i_es_tree` [F408].
- `ext4__mb_new_pa` (`ext4_mb_new_inode_pa`, `ext4_mb_new_group_pa`) reads `pa->pa_pstart`, `pa->pa_lstart`, and `pa->pa_len` in `TP_fast_assign`, and its `TP_fast_assign` along with `ext4_mb_release_inode_pa`, `ext4_mb_release_group_pa`, and `ext4_mballoc_discard` execute while holding the block group bit-spinlock (`ext4_lock_group(sb, group)`). They must never sleep, and `trace_ext4_mb_new_*_pa` must be called after initializing those `pa` fields and before publishing `pa` onto `grp->bb_prealloc_list` or `ei->i_prealloc_node` [F410, F446].
- `TP_fast_assign` blocks of `ext4_da_update_reserve_space`, `ext4_da_reserve_space`, and `ext4_da_release_space` execute under `spin_lock(&EXT4_I(inode)->i_block_reservation_lock)` to serialize reads of `EXT4_I(inode)->i_reserved_data_blocks`; callers must hold this lock and the tracepoints must not sleep [F445].
- `TRACE_EVENT(ext4_drop_inode)` runs under `inode->i_lock` (from `super_operations.drop_inode`) and must not sleep; `ext4_es_shrink*` tracepoints run from `ext4_es_count`/`ext4_es_scan` shrinker callbacks during memory reclaim and must not perform `GFP_KERNEL` allocations [F447].

## Object Lifetimes
- `TRACE_EVENT(ext4_error)` stores `function` as a raw `const char *` pointer (`__field(const char *, function)`) rather than copying it with `__string()`; callers of `trace_ext4_error()` must pass a NUL-terminated string with static storage duration (such as `__func__` or `""`), never a stack or dynamically freed buffer [F441].

## API Contracts
- In `include/trace/events/ext4.h`, every enum constant referenced directly or indirectly (such as `1UL << BH_*` or `1 << ES_*_B`) inside `__print_flags()` or `__print_symbolic()` (e.g., `show_mflags`, `show_extent_status`, `show_fc_reason`, `show_criteria`) must be wrapped in `TRACE_DEFINE_ENUM()` [F443].

## Error Handling
- `ext4_xattr_user_get` and `ext4_xattr_user_set` must check `test_opt(inode->i_sb, XATTR_USER)` and return `-EOPNOTSUPP` (not `-EINVAL` or `-ENODATA`) when disabled before delegating to `ext4_xattr_get` or `ext4_xattr_set`, while `ext4_xattr_user_list` returns a `bool` indicating if `XATTR_USER` is enabled on `dentry->d_sb` [F411].

## External Contracts
- In `include/uapi/linux/ext4.h`, ioctl commands whose `_IOR`/`_IOW` type has architecture-dependent width or alignment (`long`, `unsigned long`, or `struct ext4_new_group_input` which places `__u32 group` before unpadded `__u64` fields) require `EXT4_IOC32_*` compat definitions; `EXT4_IOC32_GROUP_ADD` (`struct compat_ext4_new_group_input`) must read each field individually in `ext4_compat_ioctl` rather than routing directly to `EXT4_IOC_GROUP_ADD`. New ioctls must use fixed-width types and 8-byte-aligned `__u64` fields [F424, F438].
- New `EXT4_IOC_*` commands in `include/uapi/linux/ext4.h` using type `'f'` must not use command numbers 10, 11 (reserved for FIEMAP) or 19 through 39 (reserved for fscrypt) [F453].
- In `struct move_extent` (`EXT4_IOC_MOVE_EXT`), `orig_start`, `donor_start`, `len`, and `moved_len` are in filesystem blocks (not bytes); `moved_len` must be initialized to `0` before `ext4_move_extents` and copied back to userspace even if `ext4_move_extents` returns an error [F416].
- For `struct fsuuid` (`EXT4_IOC_GETFSUUID`, `EXT4_IOC_SETFSUUID`), `fsu_flags` must be `0` (else return `-EINVAL`), and because `sizeof(struct fsuuid)` covers only the 8-byte header (`fsu_len`, `fsu_flags`), the flexible array `fsu_uuid[]` must be copied separately after validating `fsu_len` (`0` or `>= UUID_SIZE` for get; `== UUID_SIZE` for set) [F417].
- `struct ext4_tune_sb_params` contains explicit padding (`pad_1`, `pad_2`, `pad[64]`) and `mount_opts[64]`; `ext4_ioctl_get_tune_sb` and `ext4_sb_setparams` must zero the struct (`memset`) and use `strscpy_pad` when copying between `mount_opts` and `es->s_mount_opts` [F426].
- `EXT4_IOC_GETSTATE` must only expose UAPI `EXT4_STATE_FLAG_*` bitmasks by translating individual `ext4_test_inode_state(inode, EXT4_STATE_*)` bits, never copying raw `EXT4_I(inode)->i_state_flags` directly to userspace [F454].
- Any new `EXT4_IOC_CHECKPOINT_FLAG_*` bit in `include/uapi/linux/ext4.h` must be ORed into `EXT4_IOC_CHECKPOINT_FLAG_VALID`, and any new `EXT4_TUNE_FL_*` flag must be ORed into `TUNE_OPS_SUPPORTED` in `fs/ext4/ioctl.c` [F455].

## Known Pitfalls
- In `include/trace/events/ext4.h`, `ext4_mballoc_alloc` must assign `result_*` fields from `ac->ac_f_ex` (saved before preallocation modifies `ac->ac_b_ex`), whereas `ext4_mballoc_prealloc` must assign `result_*` fields from `ac->ac_b_ex` (`ac->ac_f_ex` is unpopulated when using an existing preallocation) [F430].
- In `ext4_ext_convert_to_initialized_fastpath`, `TP_fast_assign` receives `map`, unwritten extent `ux`, and initialized extent `ix`, and must assign `m_*` from `map`, `u_*` from `ux`, and `i_*` from `ix` without mixing `ux` and `ix` [F433].
- In `include/trace/events/ext4.h`, 64-bit divisions inside `TP_fast_assign` (such as `u64 scan_time` in `ext4_es_shrink`, or any `u64`, `__u64`, `loff_t`, or `ext4_fsblk_t`) must use helpers like `div_u64()` or `do_div()` rather than `/` or `%` to avoid undefined `__udivdi3` link errors on 32-bit builds [F435].
- `EXT4_IOC32_*` macros are defined in both `include/uapi/linux/ext4.h` and `#if defined(__KERNEL__) && defined(CONFIG_COMPAT)` in `fs/ext4/ext4.h`; any change to `EXT4_IOC32_*` must keep both files synchronized to avoid macro redefinition warnings or mismatches [F457].
