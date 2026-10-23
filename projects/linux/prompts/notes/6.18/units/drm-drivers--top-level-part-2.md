# DRM DRIVERS — (top level) (part 2)

This component provides core DRM C headers (rectangles, simple KMS pipes, suballocation, syncobjs, vblanks/workers, VMA offsets, writeback connectors, SPSC queues, task barriers, VGA arbitration), DRM/driver UAPI headers (KMS/DRM ioctls, FourCC modifiers, OMAP/VGEM/VirtGPU ioctls), and Rust `kernel::drm` abstractions.

## Architecture & Data Flow

- `Device<T>::new` must pass `Kmalloc::aligned_layout(Layout::new::<Self>()).size()` and `mem::offset_of!(Self, dev)` to `bindings::__drm_dev_alloc` so `T::Data` is aligned and `from_drm_device` can use `container_of!(..., Self, dev)`. [F236]
- In `rust/kernel/drm/`, `impl drm::Driver for T` requires `<T::File as DriverFile>::Driver == T` (as `Device<T>::VTABLE` wires `File::<T::File>::open_callback` casting `raw_dev` via `Device::from_raw`), and any `T: DriverObject` in `gem::Object<T>` requires `<T::Driver as drm::Driver>::Object == gem::Object<T>` (for `IntoGEMObject::from_raw` downcasts in `open_callback`/`close_callback`). [F256, F261]
- `Driver::IOCTLS` must use `declare_drm_ioctls!` with entries contiguous from `DRM_COMMAND_BASE` (`i == _IOC_NR(cmd)`), `size_of::<uapi::$struct>() == _IOC_SIZE(cmd)`, and each `$func` taking `&drm::Device<Self>` and `&drm::File<Self::File>` matching `Self` (as `Device::from_raw`/`File::from_raw` infer container types unchecked). [F217, F255]
- `struct drm_rect` must match UAPI `struct drm_mode_rect`'s exact `s32` layout (`x1, y1, x2, y2`). `(x1, y1)` are inclusive and `(x2, y2)` exclusive (`width = x2 - x1`, `height = y2 - y1`); `DRM_RECT_INIT`/`drm_rect_init` take `(x, y, width, height)`, `DRM_RECT_ARG` expands width/height before `x1`/`y1`, and `drm_rect_fp_to_int()` must set `dst->x2 = (src->x1 >> 16) + (drm_rect_width(src) >> 16)` (not `src->x2 >> 16`). [F198, F218, F249]
- In `struct drm_suballoc`, `soffset` is inclusive and `eoffset` is exclusive (`soffset + size`; `drm_suballoc_size()` is `eoffset - soffset`). `DRM_SUBALLOC_MAX_QUEUES` must be a power of two so `drm_suballoc_free()` can index `sa_manager->flist` via `fence->context & (DRM_SUBALLOC_MAX_QUEUES - 1)`. [F220, F221]
- `drm_writeback_connector::encoder` is only initialized by `drm_writeback_connector_init`; when using `drm_writeback_connector_init_with_encoder` or `drmm_writeback_connector_init`, an external `drm_encoder` is attached and `wb_connector->encoder` is invalid. [F202]

## Synchronization

- `spsc_queue_push` must set `node->next = NULL` before `atomic_long_xchg(&queue->tail, (long)&node->next)` and keep preemption disabled across `atomic_long_xchg` and `WRITE_ONCE(*tail, node)`. In `spsc_queue_pop`, if `READ_ONCE(node->next)` is NULL and `atomic_long_cmpxchg(&queue->tail, (long)&node->next, (long)&queue->head)` fails, it must spin with `smp_rmb()` until `READ_ONCE(node->next)` is non-NULL and write it to `queue->head`. [F175, F176]
- Never dereference `drm_syncobj.fence` directly: lockless readers must hold a `drm_syncobj` reference and use `drm_syncobj_fence_get()` (`dma_fence_get_rcu_safe`), while updates to `fence`, `cb_list`, and `ev_fd_list` must hold `drm_syncobj.lock`. [F177]
- `drm_vblank_crtc.last` is protected by `drm_device.vbl_lock`, `drm_vblank_crtc.hwmode` is only valid while `drm_vblank_crtc.enabled` is true, and `drm_vblank_crtc.count` must be read via `drm_crtc_vblank_count()` or `drm_crtc_vblank_count_and_time()` (never `atomic64_read`) to pair with `drm_crtc_handle_vblank()` seqlock barriers. [F179, F213]
- `drm_vma_offset_lock_lookup(mgr)` takes `read_lock(&mgr->vm_lock)` (atomic context): only call `drm_vma_offset_lookup_locked`/`drm_vma_offset_exact_lookup_locked`, do not sleep or call `add`/`remove`, and pin the returned object's refcount before `drm_vma_offset_unlock_lookup(mgr)`. `drm_vma_node_unmap()` reads `node->vm_node` unlocked and must be externally serialized against `drm_vma_offset_remove()`. [F180, F181]
- In `struct drm_writeback_connector`, `job_lock` protects `job_queue` while `fence_lock` protects fences in `fence_context`. [F182]
- `task_barrier_enter()`, `task_barrier_exit()`, and `task_barrier_full()` sleep via `down()` on `enter_turnstile`/`exit_turnstile` and require sleepable process context. Every `task_barrier_enter` must pair with `task_barrier_exit`, and `tb->n` must not change via `task_barrier_add_task`/`task_barrier_rem_task` while tasks are in the barrier. [F210, F281]

## Object Lifetimes

- In `Device::new`, decode `bindings::__drm_dev_alloc` with `from_err_ptr`, and if `data.__pinned_init(raw_data)` fails, call `bindings::drm_dev_put`. In `Device::release`, only call `core::ptr::drop_in_place` without freeing the pointer (`KBox::from_raw`/`kfree`), as the C DRM core frees the `__drm_dev_alloc` container after `release`. [F164, F186]
- `Registration::new_foreign_owned` requires `drm.as_ref().as_raw() == dev.as_raw()` (or returns `Err(EINVAL)` before `Registration::new`). `Registration::new` converts `drm.into()` into an owned `ARef<drm::Device<T>>` after `bindings::drm_dev_register` succeeds, keeping it alive until `Registration::drop` calls `bindings::drm_dev_unregister` (even if `devres::register` fails). [F165, F187]
- In `gem::Object::<T>::new`, set `(*obj.as_raw()).funcs = &Self::OBJECT_FUNCS` before `bindings::drm_gem_object_init`, and call `KBox::into_raw` and `ARef::from_raw` only after `drm_gem_object_init` succeeds (adopting refcount 1 on success, or dropping `Pin<KBox<Self>>` on error). `Object::<T>::free_callback` must call `bindings::drm_gem_object_release` before dropping `KBox::from_raw(this)`. [F167, F268]
- `AlwaysRefCounted::dec_ref` for `drm::Device<T>` and `IntoGEMObject` must extract the raw pointer and end any `&Self` reference lifetime before calling `bindings::drm_dev_put` or `bindings::drm_gem_object_put`, which may synchronously free `Self`. [F264]
- `File<T>::inner` and `File<T>::postclose_callback` assume `(*self.as_raw()).driver_priv` holds a valid `Pin<KBox<T>>` from `File<T>::open_callback`; never call `file.inner()` before `open_callback` sets `driver_priv`, after `postclose_callback`, or on a foreign `drm_file`. [F265]

## API Contracts

- When `vga_get`, `vga_get_interruptible`, or `vga_get_uninterruptible` returns `0`, release resources with `vga_put(pdev, rsrc)`; never call `vga_put` if `vga_get*` returns non-zero. [F173]
- `Registration::new_foreign_owned()` and `gem::Object::new()` allocate with `GFP_KERNEL` (sleepable context only), and `gem::Object::new` requires `size` to be `PAGE_SIZE`-aligned in bytes. [F189, F283]
- `drm_vma_offset_manager_init`, `drm_vma_offset_lookup_locked`, `drm_vma_offset_add`, `drm_vma_node_start`, and `drm_vma_node_size` use page units, whereas `drm_vma_node_offset_addr` returns a byte offset (`<< PAGE_SHIFT`). Use `drm_vma_offset_exact_lookup_locked()` (`node->vm_node.start == start`) when an exact base offset is required (e.g. `mmap`), and never call `drm_vma_node_reset` on a node still in a manager or with active `vm_files`. [F191, F211, F243]
- `drm_suballoc_new(sa_manager, size, gfp, intr, align)` requires `0 < size <= sa_manager->size` and `align <= sa_manager->align` (`0` uses `sa_manager->align`), or it triggers `WARN_ON_ONCE` and returns `ERR_PTR(-EINVAL)`. [F194]
- `drm_syncobj_find_fence` only accepts `0` or `DRM_SYNCOBJ_WAIT_FLAGS_WAIT_FOR_SUBMIT` in `flags` (returning `-EINVAL` otherwise); `DRM_SYNCOBJ_WAIT_FLAGS_WAIT_FOR_SUBMIT` requires sleepable context with no locks held. [F197]
- `drm_rect_visible()` requires `width > 0 && height > 0`, `drm_rect_overlap()` requires strict `>` on `x2`/`y2`, and `drm_rect_downscale()` requires non-zero `horz`/`vert`. `drm_rect_clip_scaled`, `drm_rect_calc_hscale`, and `drm_rect_calc_vscale` require 16.16 fixed-point `src` and integer `dst` (returning `0` if `dst` size is `0`, `-EINVAL` if negative, `-ERANGE` if out of bounds). Pair `DRM_RECT_FMT` with `DRM_RECT_ARG` and `DRM_RECT_FP_FMT` with `DRM_RECT_FP_ARG`. [F199, F244, F248, F258]
- Call `drm_crtc_vblank_off` when disabling a CRTC and `drm_crtc_vblank_on` (or `_on_config`) when enabling it. Call `drm_crtc_set_max_vblank_count` only when `dev->max_vblank_count == 0` and `vblank->inmodeset != 0` (before `drm_crtc_vblank_on`), with `drm_crtc_funcs.get_vblank_counter` implemented if non-zero. [F201, F270]
- Verify `connector->connector_type == DRM_MODE_CONNECTOR_WRITEBACK` before calling `drm_connector_to_writeback()`. Set writeback framebuffers via `drm_writeback_set_fb(conn_state, fb)` rather than assigning `conn_state->writeback_job->fb` directly. [F203, F257]
- `drm_timeout_abs_to_jiffies(timeout_nsec)` takes an absolute `ktime_get()` (`CLOCK_MONOTONIC`) ns timestamp (not a relative duration), returning `0` if `<= 0` or past, and clamping to `MAX_SCHEDULE_TIMEOUT - 1`. Only use `EXPORT_SYMBOL_FOR_TESTS_ONLY(x)` for `drmselftests` symbols (`CONFIG_DRM_EXPORT_FOR_TESTS`). [F205, F295]
- `drm_simple_display_pipe_funcs.mode_valid` must only check invariant hardware constraints and never read mutable `crtc->state` or `plane->state`. Non-NULL `format_modifiers` passed to `drm_simple_display_pipe_init()` must end with `DRM_FORMAT_MOD_INVALID`. [F215, F223]
- Callers of `AMD_FMT_MOD_SET(field, value)` must ensure `value` fits within `AMD_FMT_MOD_##field##_MASK` because the macro shifts without masking. [F246]

## Error Handling

- `BaseObject::lookup_handle` must map a null return from `bindings::drm_gem_object_lookup` to `Err(ENOENT)` before calling `Self::from_raw` and `ARef::from_raw`, as `drm_gem_object_lookup` returns `NULL` (not `ERR_PTR`). [F188]
- `declare_drm_ioctls!` converts the handler's `Result<u32>` to `c_int` via `i.try_into().unwrap_or(ERANGE.to_errno())`, so `Ok(i)` with `i > i32::MAX as u32` fails with `-ERANGE` (write u32 outputs to `data` and return `Ok(0)`). [F190]
- `drm_suballoc_new` and `drm_get_panel_backlight_quirk` return `ERR_PTR` on failure/no-match and never `NULL` (check with `IS_ERR`/`IS_ERR_OR_NULL`), whereas `drm_get_panel_orientation_quirk` returns `DRM_MODE_PANEL_ORIENTATION_UNKNOWN`, and `drm_syncobj_find` / `drm_syncobj_fence_get` return `NULL` (never `ERR_PTR`, unlike `drm_syncobj_find_fence` which returns negative errno). [F193, F196, F204]
- `drm_vblank_work_schedule` returns `1` if scheduled/rescheduled, `0` if already at `count`, `work->cancelling`, or `vblank->inmodeset`, and `< 0` only on `drm_vblank_get` error. [F200]

## External Contracts

- GEM handles on a `drm_file` are not refcounted: `DRM_IOCTL_PRIME_FD_TO_HANDLE` deduplicates repeated imports to the same handle, whereas `DRM_IOCTL_MODE_GETFB2` allocates fresh handles, so call `DRM_IOCTL_GEM_CLOSE` only once per unique handle. [F174]
- In `struct drm_omap_gem_new`, `union omap_gem_size size` uses `size.tiled` when `flags & OMAP_BO_TILED_MASK` is non-zero and `size.bytes` when zero. [F206]
- In `struct drm_mode_fb_cmd2`, when `DRM_MODE_FB_MODIFIERS` is set, all used planes must have identical `modifier[i]` (and `DRM_IOCTL_MODE_GETFB2` only populates `modifier` when `DRM_MODE_FB_MODIFIERS` is set). Unused plane entries in `handles`, `pitches`, `offsets`, and `modifier` must be `0`; `DRM_IOCTL_MODE_GETFB2` zeroes `handles` unless DRM master or `CAP_SYS_ADMIN` (check `pitches[i]` for valid planes). [F207, F225]
- In `include/uapi/drm/`, represent user pointers as `__u64` and pad all fields to natural alignment (`pad`/`__pad`) for identical 32/64-bit layout. [F224]
- `drm_mode_get_plane::possible_crtcs` bits, `drm_pending_vblank_event::pipe`, and `drm_vblank_crtc::pipe` use 0-based `drm_crtc_index()`, never KMS object IDs (`drm_crtc::base.id`). In `struct drm_format_modifier`, bit `b` of `formats` maps to plane format index `offset + b`. [F227, F228]
- In `struct drm_mode_fb_dirty_cmd`, `num_clips <= DRM_MODE_FB_DIRTY_MAX_CLIPS` (256), and `DRM_MODE_FB_DIRTY_ANNOTATE_COPY` pairs source/destination rects (`num_clips / 2` regions; `num_clips` must be even). [F229]
- In `drm_fourcc.h`, 64-bit modifiers pack 8-bit `DRM_FORMAT_MOD_VENDOR_*` in bits 63:56 and 56-bit value in bits 55:0 via `fourcc_mod_code`, with reserved bits (e.g. `AMD_FMT_MOD` bits 55:36) zeroed; modifiers must not alias or encode pitch-alignment subsets, and `DRM_FORMAT_MOD_GENERIC_*` must match the original value. [F232, F294]
- In `drm_mode.h`, `drm_mode_modeinfo::flags` bits 0-13 must match XRandR `RR_*` bits and new mode flags require a `DRM_CLIENT_CAP_*` gate. `drm_color_ctm::matrix` uses S31.32 sign-magnitude (bit 63 sign, bits 62:0 magnitude), not two's complement. [F233, F234]
- Modeset-mutating ioctls must set `drm::ioctl::MASTER` (`DRM_MASTER`); render-node ioctls must set `drm::ioctl::RENDER_ALLOW` (`DRM_RENDER_ALLOW`). [F293]

## Known Pitfalls

- `drm_fourcc_canonicalize_nvidia_format_mod()` must check `!(modifier & 0x10) || (modifier & (0xff << 12))` before ORing `0xfe << 12`. Broadcom modifiers (`fourcc_mod_broadcom_mod`/`param`) store `params` in bits `[55:8]` (cast to `int`) and base `val` in bits `[7:0]`, requiring `val <= 0xff` and `params <= INT_MAX`. [F240, F247]
- Mask multi-bit/zero UAPI fields before equality checks: in `omap_drm.h`, `OMAP_BO_TILED_32` (`0x300`) is `OMAP_BO_TILED_8 | OMAP_BO_TILED_16` and `OMAP_BO_CACHED` is `0` (mask with `OMAP_BO_TILED_MASK`/`OMAP_BO_CACHE_MASK`); in `drm_mode.h`, check `(prop->flags & DRM_MODE_PROP_EXTENDED_TYPE) == type` for `DRM_MODE_PROP_OBJECT`/`DRM_MODE_PROP_SIGNED_RANGE`. [F241, F242]
- Never use `drm_can_sleep()` in new code, as `in_atomic()` does not reliably detect all atomic contexts. [F282]
