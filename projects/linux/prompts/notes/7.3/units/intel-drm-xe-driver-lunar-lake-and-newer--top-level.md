# INTEL DRM XE DRIVER (Lunar Lake and newer) — (top level)

This component contains the hardware workaround tables and evaluation engine glue (`xe_wa.c`), the `DRM_IOCTL_XE_WAIT_USER_FENCE` ioctl implementation (`xe_wait_user_fence.c`), the Write Once Protected Content Memory layout calculator and register programming (`xe_wopcm.c`), and the complete userspace API header (`include/uapi/drm/xe_drm.h`).

## Architecture & Data Flow
- `XE_GT_WA` must only be called after `xe_wa_gt_init` and `xe_wa_process_gt_oob` have allocated `gt->wa_active.oob` and set `gt->wa_active.oob_initialized`; similarly, `XE_DEVICE_WA` and `XE_DEVICE_WA_DISABLE` require `xe_wa_device_init` and `xe_wa_process_device_oob` to have initialized `xe->wa_active.oob` and `xe->wa_active.oob_initialized`. [F1433]
- `__wopcm_regs_locked` must require both `GUC_WOPCM_SIZE_LOCKED` in `GUC_WOPCM_SIZE` and `GUC_WOPCM_OFFSET_VALID` in `DMA_GUC_WOPCM_OFFSET` to be set before treating WOPCM registers as locked; when locked, `xe_wopcm_init` must set `wopcm->size = MAX_WOPCM_SIZE` (not `xe_wopcm_size(xe)`), jump to `check:` to validate the locked layout, and skip `__wopcm_init_regs`. [F1480]

## Synchronization
- In `xe_wait_user_fence_ioctl`, `do_compare` (`copy_from_user`) and `LNL_FLUSH_WORKQUEUE(xe->ordered_wq)` must execute in sleepable process context outside `xe->ufence_wq.lock` using `woken_wake_function` and `wait_woken`. Broken by: moving `do_compare` into a custom `wait_queue_entry_t` wake callback on `xe->ufence_wq`, which runs under `ufence_wq.lock` in hardirq context when `xe_hw_engine_handle_irq` calls `wake_up_all`. [F1501]
- Before `xe_wait_user_fence_ioctl` returns `-ETIME` when `timeout` reaches zero, it must flush `xe->ordered_wq` via `LNL_FLUSH_WORKQUEUE(xe->ordered_wq)` and re-run `do_compare`, and it must never hold any lock across `LNL_FLUSH_WORKQUEUE(xe->ordered_wq)` that could be acquired by work items on `xe->ordered_wq`. [F1431, F1502]
- When `args->exec_queue_id` is non-zero, `xe_wait_user_fence_ioctl` must pin `q` via `xe_exec_queue_lookup` before the wait loop, revalidate `q->ops->reset_status(q)` on every iteration when `do_compare` does not match, and drop `q` via `xe_exec_queue_put` on all exit paths. [F1430]

## Object Lifetimes
- In `xe_wait_user_fence_ioctl`, the stack-allocated waitqueue entry `w_wait` registered on `xe->ufence_wq` via `add_wait_queue` must be unlinked via `remove_wait_queue(&xe->ufence_wq, &w_wait)` before returning; all exits from the `for (;;)` wait loop must `break` to the cleanup path rather than `return` directly. [F1426]

## API Contracts
- `xe_wopcm_init` requires its `struct xe_wopcm *wopcm` argument to be embedded at `gt->uc.wopcm` (because `wopcm_to_gt` and `wopcm_to_xe` unconditionally use `container_of(wopcm, struct xe_gt, uc.wopcm)`), requires the caller to hold `XE_FW_GT` forcewake, and requires `gt->uc.guc.fw` to have a non-zero upload size (returning `-EINVAL` if `!guc_fw_size`); it must also validate the layout via `__check_layout` (returning `-E2BIG` on failure) before populating `wopcm->guc` or calling `__wopcm_init_regs`. [F1441, F1490]
- `__wopcm_init_regs` requires non-zero `wopcm->guc.base` and `wopcm->guc.size` within `GUC_WOPCM_OFFSET_MASK` and `GUC_WOPCM_SIZE_MASK`, and must program `GUC_WOPCM_SIZE` and `DMA_GUC_WOPCM_OFFSET` (including `HUC_LOADING_AGENT_GUC` if HuC FW is available) via `xe_mmio_write32_and_verify` to confirm `GUC_WOPCM_SIZE_LOCKED` and `GUC_WOPCM_OFFSET_VALID`. [F1500]
- In `do_compare`, both the user value read from `addr` (`rvalue`) and `value` must be bitwise-ANDed with `mask` before evaluating any `DRM_XE_UFENCE_WAIT_OP_*` operator, matching the `(*addr & MASK) OP (VALUE & MASK)` contract of `struct drm_xe_wait_user_fence`. [F1472]

## Error Handling
- `do_compare` in `drivers/gpu/drm/xe/xe_wait_user_fence.c` returns `< 0` on error (`-EFAULT` or `-EINVAL`), `0` on match, and `1` on mismatch; any wait-loop exit path in `xe_wait_user_fence_ioctl` taken after a mismatch (`err == 1`) must overwrite `err` with a negative errno before breaking. [F1434]

## External Contracts
- `xe_wait_user_fence_ioctl` must reject any `struct drm_xe_wait_user_fence` input where `extensions`, `pad`, `pad2`, `reserved[0]`, `reserved[1]`, `flags & ~VALID_FLAGS`, or `addr & 0x7` is non-zero, or where `op > MAX_OP`, returning `-EINVAL` before dereferencing `addr` or looking up `exec_queue_id`. [F1506]
- In `xe_wait_user_fence.c`, `to_jiffies_timeout` must overwrite a negative `args->timeout` with `MAX_SCHEDULE_TIMEOUT` (so post-wait subtraction does not clamp it to `0` on signal restart), and `xe_wait_user_fence_ioctl` must subtract elapsed nanoseconds from `args->timeout` (clamped to `>= 0`) only when `DRM_XE_UFENCE_WAIT_FLAG_ABSTIME` is not set. [F1455]
- In `struct drm_xe_vm_bind`, the union between `bind` (`struct drm_xe_vm_bind_op`) and `vector_of_binds` (`__u64` user pointer) is discriminated by `num_binds`: `bind` is used when `num_binds == 1` and `vector_of_binds` is used when `num_binds > 1`. [F1445]
- In `include/uapi/drm/xe_drm.h`, the `DRM_XE_*` ioctl command offsets (`0x00` through `0x0d`) must remain compact with no holes and stay synchronized with `DRM_IOCTL_XE_*` and the `xe_ioctls` table in `drivers/gpu/drm/xe/xe_device.c`. [F1458]

## Known Pitfalls
- `to_jiffies_timeout` must capture `nsecs_to_jiffies(args->timeout)` and `drm_timeout_abs_to_jiffies(args->timeout)` in an `unsigned long long t` and clamp against `MAX_SCHEDULE_TIMEOUT` before assigning to `long timeout`, because `nsecs_to_jiffies` can overflow a signed 32-bit `long`. [F1481]
- To prevent 32-bit unsigned underflow or wraparound, `xe_wopcm_init` must clamp `guc_wopcm_base` to `min(guc_wopcm_base, wopcm->size - ctx_rsvd)` before computing `guc_wopcm_size = wopcm->size - ctx_rsvd - guc_wopcm_base`, and `__check_layout` must verify `guc_wopcm_base >= size` before evaluating `guc_wopcm_size > size - guc_wopcm_base`. [F1484]
- In `xe_wa.c`, `engine_was` entries for registers shared across the render/compute reset domain must match `FUNC(xe_rtp_match_first_render_or_compute)` (not `ENGINE_CLASS(RENDER)`, which fails when RCS is fused off), and actions on non-readable registers must disable read verification via `.read_mask = 0`, `XE_RTP_NOCHECK`, or `FIELD_SET_NO_READ_MASK`. [F1496]
