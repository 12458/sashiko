# RADEON and AMDGPU DRM DRIVERS — drivers/gpu/drm/amd/amdgpu (part 12)

Implements GC/GFX IP drivers for GFX v11 (`gfx_v11_0.c`, `gfx_v11_0_3.c`, RDNA3) and GFX v12 (`gfx_v12_0.c`, RDNA4), handling CP/RLC/IMU firmware, PM4 ring emission, MQDs, usermode queues, interrupts, GFXOFF/CG/PG, cleaner shaders, and queue/pipe resets.

## Architecture & Data Flow

- Changing PM4 dword counts in ring callbacks (`emit_vm_flush`, `emit_fence`, `emit_gfx_shadow`, `emit_cleaner_shader`, `emit_mem_sync`, `emit_ib`) requires updating `.emit_frame_size`/`.emit_ib_size` in `gfx_v11_0_ring_funcs_*`/`gfx_v12_0_ring_funcs_*`; KIQ packet sizes in `gfx_v11_0_kiq_pm4_funcs`/`gfx_v12_0_kiq_pm4_funcs` must match `gfx11_kiq_*`/`gfx_v12_0_kiq_*` dwords. [F946, F947]
- `gfx_v11_0_get_csb_size` must match the dwords written by `gfx_v11_0_get_csb_buffer` and `gfx_v11_0_cp_gfx_start` (`gfx_v12_0_get_csb_size` must match `gfx_v12_0_get_csb_buffer`). `gfx11_cs_data`/`gfx12_cs_data` must end `cs_section_def` with `.section == NULL` and each `cs_extent_def` with `.extent == NULL`. [F948, F970]
- `ring->wptr` is a monotonic 64-bit dword counter; `gfx_v11_0_ring_emit_init_cond_exec`, `gfx_v11_0_ring_emit_gfx_shadow`, and `gfx_v12_0_ring_emit_init_cond_exec` must mask `ring->wptr & ring->buf_mask` before indexing `ring->ring[]` or computing byte offsets (`<< 2`) into `ring->ring_obj`. [F968]
- In `gfx_v11_0_ring_emit_gfx_shadow`, postfix (`!shadow_va`) must set `ring->set_q_mode_ptr = &ring->ring[ring->set_q_mode_offs]` before `ring->set_q_mode_offs = offs`; `gfx_v11_0_ring_emit_vm_flush` must reset `set_q_mode_offs = 0` and `set_q_mode_ptr = NULL`. [F991]
- In `gfx_v1*_0_kgq_init_queue` and `gfx_v1*_0_kcq_init_queue`, initial boot saves `mqd` to `mqd_backup[mqd_idx]`; reset/resume must restore `mqd` from `mqd_backup[mqd_idx]`, zero `ring->wptr` and `*ring->wptr_cpu_addr`, and call `amdgpu_ring_clear_ring(ring)`. [F955]
- `gfx_v11_0` and `gfx_v12_0` only have 1 MEC (`adev->gfx.mec.num_mec = 1`, `mec2_fw = NULL`), and `*_get_cpc_int_cntl` / `*_set_compute_eop_interrupt_state` must only touch `me == 1` (leaving other MECs to `amdkfd`). [F953, F1009]
- `gfx_v11_0_init_gds_vmid` must only clear GDS/GWS/OA registers for `vmid >= 1` (preserving `vmid = 0` for HWS), and `gfx_v1*_0_constants_init` must only program `regSH_MEM_BASES` for `i != 0`. [F952]
- `gfx_v1*_0_alloc_ip_dump`, `gfx_v1*_ip_dump`, and `gfx_v1*_ip_print` must use identical ME/MEC pipe/queue counts and `gc_cp_reg_list_*`/`gc_gfx_queue_reg_list_*` sizes when sizing and indexing `ip_dump_compute_queues` and `ip_dump_gfx_queues`. [F957]
- In `gfx_v1*_0_rlc_backdoor_autoload_copy_ucode`, verify `id > *_FIRMWARE_ID_INVALID && id < *_FIRMWARE_ID_MAX` before indexing `rlc_autoload_info[id]`, clamp `fw_size` to `rlc_autoload_info[id].size`, and in v11 use `1ULL << id` for `*(uint64_t *)fw_autoload_mask` (`SOC21_FIRMWARE_ID_MAX == 37`). [F965, F992]
- `regRLC_CGTT_MGCG_OVERRIDE` bits are active-low (`0` = CG enabled, `1` = overridden), whereas `CGCG_EN`/`CGLS_EN` in `regRLC_CGCG_CGLS_CTRL*` are active-high; in `gfx_v1*_0_rlc_smu_handshake_cntl`, `RLC_PG_CNTL__SMU_HANDSHAKE_DISABLE_MASK` is active-low (`!enable` sets the bit to disable GFXOFF). [F977, F978]
- In `gfx_v1*_0_get_*_active_bitmap*`, `CC_*` and `GC_USER_*` registers have `1` = disabled: OR both masks, invert (`~`), and mask with `amdgpu_gfx_create_bitmask`. WGP bitmask width is `max_cu_per_sh >> 1`, and each active WGP bit `wgp_idx` (`0..15`) maps to two CUs via `3 << (2 * wgp_idx)`. [F980, F997]

## Synchronization

- `gfx_v1*_0_select_se_sh` writes `regGRBM_GFX_INDEX` unlocked: callers must hold `adev->grbm_idx_mutex` and restore broadcast (`0xffffffff, 0xffffffff, 0xffffffff, 0`) before unlocking. `gfx_v1*_0_select_me_pipe_q` and `soc21_grbm_select`/`soc24_grbm_select` write `regGRBM_GFX_CNTL` unlocked: runtime callers must hold `adev->srbm_mutex` and reset to `(0, 0, 0, 0)`. [F921, F922]
- In `gfx_v1*_reset_gfx_pipe` and `gfx_v1*_0_reset_compute_pipe`, acquire RLC safe mode (`set_safe_mode`), `adev->srbm_mutex`, and `soc21_grbm_select`/`soc24_grbm_select` in order, and release in reverse (`grbm_select(0,0,0,0)`, `mutex_unlock`, `unset_safe_mode`) on all paths. [F916]
- All `amdgpu_irq_src_funcs.set` callbacks (`gfx_v1*_0_set_eop_interrupt_state`, `*_priv_reg_fault_state`, `*_bad_op_fault_state`, `*_priv_inst_fault_state`) run under `adev->irq.lock` (`spin_lock_irqsave`) and must not sleep, call `amdgpu_gfx_off_ctrl`, or lock `srbm_mutex`. [F1024]
- In `gfx_v1*_0_eop_irq`, `xa_load(&adev->userq_xa, doorbell_offset)` returns an unreferenced `amdgpu_userq_fence_driver` pointer; keep `xa_lock_irqsave(&adev->userq_xa)` held across `amdgpu_userq_fence_driver_process(fence_drv)`. [F920]
- In `gfx_v1*_0_ring_preempt_ib`, unlock `kiq->ring_lock` if `amdgpu_ring_alloc` fails, and after `amdgpu_ring_set_preempt_cond_exec(ring, false)`, always restore `amdgpu_ring_set_preempt_cond_exec(ring, true)` even on fence timeout. [F919]
- `gfx_v1*_0_reset_kgq` and `gfx_v1*_0_reset_kcq` must call `amdgpu_ring_reset_helper_begin(ring, timedout_fence)` before resetting or calling `*_init_queue(ring, true)`, and finish with `amdgpu_ring_reset_helper_end`. [F927]
- When `ring->use_doorbell` is set in `gfx_v1*_0_ring_set_wptr_gfx` and `gfx_v1*_0_ring_set_wptr_compute`, store `ring->wptr` to `ring->wptr_cpu_addr` via `atomic64_set` before `WDOORBELL64(ring->doorbell_index, ring->wptr)`. [F929]
- In `gfx_v11_0_soft_reset`, hold both `adev->gfx.reset_sem_mutex` and `gfx_v11_0_request_gfx_index_mutex(adev, true)` around writing `regCP_VMID_RESET`, and read `regCP_VMID_RESET` three times before `gfx_v11_0_request_gfx_index_mutex(adev, false)`. [F954]
- `gfx_v11_0_get_gpu_clock_counter` must read the upper timestamp (`regCP_MES_MTIME_HI` on SR-IOV VF under `gpu_clock_mutex` with GFXOFF disabled, or `regGOLDEN_TSC_COUNT_UPPER` under `preempt_disable()`) before and after the lower 32-bit register, re-reading the lower register if the upper changed. [F960]

## Object Lifetimes

- In `gfx_v1*_0_ring_test_ib`, release resources from `amdgpu_device_wb_get`, `amdgpu_ib_get`, and `amdgpu_ib_schedule` in reverse order (`amdgpu_ib_free` and `dma_fence_put(f)` at `err2`, `amdgpu_device_wb_free` at `err1`) on all paths. [F908]
- Kernel BOs created via `amdgpu_bo_create_reserved` (`hpd_eop_obj`, `pfp_fw_obj`, `pfp_fw_data_obj`, `me_fw_obj`, `me_fw_data_obj`, `mec_fw_obj`, `mec_fw_data_obj`) must call `amdgpu_bo_kunmap` and `amdgpu_bo_unreserve` after CPU population. [F909]
- In `gfx_v11_0_sw_init`, if `amdgpu_gfx_cleaner_shader_sw_init` fails, reset `adev->gfx.enable_cleaner_shader = false`. In `gfx_v1*_0_sw_fini`, all BOs, rings, firmware, and `kcalloc` IP dump buffers must be manually freed, and when `!adev->enable_mes_kiq`, call `amdgpu_gfx_kiq_free_ring(&adev->gfx.kiq[0].ring)` before `amdgpu_gfx_kiq_fini(adev, 0)`. [F910, F911, F1022]
- `gfx_v1*_0_hw_fini` must call `cancel_delayed_work_sync(&adev->gfx.idle_work)` before releasing IRQs (`amdgpu_irq_put`, `*_set_userq_eop_interrupts`) and disabling queues/CP; in `gfx_v11_0_hw_fini`, if `amdgpu_sriov_vf(adev)` is true, return after unmapping KGQ/KCQ/KIQ without calling `gfx_v11_0_cp_enable(adev, false)` or `gart_disable`. [F912, F913]
- In `gfx_v11_0_get_gpu_clock_counter`, `gfx_v11_0_update_spm_vmid`, and `gfx_v1*_ip_dump`, call `amdgpu_gfx_off_ctrl(adev, false)` before locking `gpu_clock_mutex`/`srbm_mutex` and pair with `amdgpu_gfx_off_ctrl(adev, true)` after unlocking. In `gfx_v11_0_set_powergating_state` (GC 11.0.1, 11.0.4, 11.5.x), call `amdgpu_gfx_off_ctrl(adev, false)` before `gfx_v11_cntl_pg(adev, false)` when un-gating, and `gfx_v11_cntl_pg(adev, true)` before `amdgpu_gfx_off_ctrl(adev, true)` when gating. [F917, F918]
- In `gfx_v1*_0_hw_init`, when `load_type` is `AMDGPU_FW_LOAD_DIRECT` or `AMDGPU_FW_LOAD_RLC_BACKDOOR_AUTO` (with `amdgpu_dpm == 1`), call `amdgpu_pm_load_smu_firmware` before `gfx_v1*_0_rlc_resume`. [F1012]

## API Contracts

- `gfx_v1*_0_get_cpg_int_cntl` and `gfx_v1*_0_get_cpc_int_cntl` return `0` when `(me, pipe)` is invalid/unmanaged; callers must check `if (cp_int_cntl_reg)` before `RREG32_SOC15_IP`/`WREG32_SOC15_IP`. In `soc21_grbm_select`, `soc24_grbm_select`, and `*_get_cpc_int_cntl`, ME 0 is GFX, compute MECs are `me = mec + 1`, and `(0, 0, 0, 0)` resets selection. [F931, F943]
- `gfx_v1*_0_get_gfx_shadow_info` only populates `shadow_info` and returns `0` when `adev->gfx.cp_gfx_shadow || skip_check`; otherwise it zeroes `*shadow_info` and returns `-ENOTSUPP` (v11) or `-EINVAL` (v12). [F932]
- Callers must zero-initialize `*no_fields` and pass `simd == 0` (`WARN_ON(simd != 0)`) to `gfx_v1*_0_read_wave_data`, and zero-initialize `*flags` before `gfx_v1*_0_get_clockgating_state` (which only ORs bits via `*flags |= ...`). [F940, F941]
- `gfx_v1*_0_ring_emit_fence` requires 8-byte `addr` alignment with `AMDGPU_FENCE_FLAG_64BIT` (`BUG_ON(addr & 0x7)`) and 4-byte otherwise (`BUG_ON(addr & 0x3)`), while `gfx_v1*_0_ring_emit_fence_kiq` forbids `AMDGPU_FENCE_FLAG_64BIT`. `gfx_v1*_0_wait_reg_mem` (when `mem_space != 0`) and `gfx_v1*_0_ring_emit_ib_*` require 4-byte aligned GPU addresses (`!(addr & 0x3)`). [F944, F972, F973]
- In `gfx_v1*_0_gfx_mqd_init` and `gfx_v1*_0_compute_mqd_init`, `prop->queue_size` is in bytes (encoded as `order_base_2(prop->queue_size / 4) - 1`), and `prop->hqd_base_gpu_addr` / `prop->eop_gpu_addr` are byte addresses that must be shifted right by 8 (`>> 8`). [F945]
- In `gfx_v11_0_cp_resume`, when `adev->gfx.disable_kq` is true, test the temporary GFX ring with `amdgpu_ring_test_ring(ring)` (not `amdgpu_ring_test_helper(ring)`, keeping `ring->sched.ready` false) and disable it via `amdgpu_gfx_disable_kgq(adev, 0)`. [F956]
- Bracket RLC CG/PG register writes in `gfx_v1*_0_update_gfx_clock_gating` and `gfx_v11_cntl_pg` with `amdgpu_gfx_rlc_enter_safe_mode(adev, 0)` and `amdgpu_gfx_rlc_exit_safe_mode(adev, 0)`. [F958]
- In `gfx_v1*_0_sw_init`, set `adev->userq_funcs` to `&userq_mes_funcs` only when `!adev->gfx.disable_uq` and PFP, ME, MEC, and MES fw versions meet minimums; set `AMDGPU_RESET_TYPE_PER_QUEUE` only when `!amdgpu_sriov_vf(adev)` and ME/MEC fw versions meet minimums. [F1008, F1010]

## Error Handling

- In `gfx_v1*_0_ring_test_ib`, store `dma_fence_wait_timeout` in `long r`, convert `r == 0` to `-ETIMEDOUT`, and propagate `r < 0` before checking `*cpu_ptr`. [F930]
- `adev->gfx.ras` is only set on `IP_VERSION(11, 0, 3)` and `NULL` elsewhere, so `gfx_v11_0_rlc_gc_fed_irq` must null-check `adev->gfx.ras` and its callback; `gfx_v11_0_3_rlc_gc_fed_irq` must null-check `ras_if` (`sdma.ras_if`/`gfx.ras_if`) and `adev->virt.ops && adev->virt.ops->ras_poison_handler`. [F933, F934]
- `gfx_v1*_0_alloc_ip_dump` leaves `ip_dump_core`, `ip_dump_compute_queues`, and `ip_dump_gfx_queues` `NULL` on `kcalloc` failure without failing `sw_init`, so `gfx_v1*_ip_dump`/`gfx_v1*_ip_print` must null-check each pointer; `gfx_v11_0_ring_emit_gfx_shadow` must check `adev->gfx.cp_gfx_shadow && ring->ring_obj` before accessing `ring->ring_obj`. [F935, F936]
- In `gfx_v1*_0_init_microcode` and `gfx_v1*_0_init_toc_microcode`, errors from `amdgpu_ucode_request` or `amdgpu_gfx_rlc_init_microcode` must jump to `out:` to `amdgpu_ucode_release` all requested firmware pointers while returning `err`. [F938]
- `gfx_v1*_0_gfx_mqd_init` and `gfx_v1*_0_compute_mqd_init` only populate a subset of MQD fields; callers must zero the MQD buffer before calling `amdgpu_ring_init_mqd` or `init_mqd`. [F939]
- In `gfx_v1*_0_handle_priv_fault`, only call `drm_sched_fault(&ring->sched)` when `!adev->gfx.disable_kq`, and `BUG()` if `me_id` is outside `0..2`. [F1042]

## External Contracts

- On GFX11/GFX12 rings, RPTR writeback (`ring->rptr_cpu_addr`) is 32-bit (`*(uint32_t *)`), while WPTR writeback (`ring->wptr_cpu_addr`) and doorbells are 64-bit (`atomic64_read`/`atomic64_set`, `WDOORBELL64`). [F975]

## Known Pitfalls

- Pointer arithmetic vs byte sizes: in `gfx_v12_0_cp_compute_load_microcode_rs64`, divide the 64KB byte stride by 4 when indexing `u32 *fw_data_ptr` (`+ i * ALIGN(fw_data_size, 64 * 1024) / 4`), but use unscaled bytes for `regCP_MEC_MDBASE_LO`/`_HI`; in `gfx_v1*_0_rlc_backdoor_autoload_copy_toc_ucode`, divide byte `size` by `sizeof(*toc_ptr)` (`/ 8` for `uint64_t *` in v11, `/ 4` for `uint32_t *` in v12); in `gfx_firmware_header_v1_0`, `jt_offset`/`jt_size` are dwords (`jt_size * 4` when subtracting from `ucode_size_bytes`, unscaled when indexing `__le32 *fw_data`); in `gfx_v11_0_ring_emit_de_meta`, pass `sizeof(de_payload) >> 2` dwords to `amdgpu_ring_write_multiple` and `PACKET3_WRITE_DATA`. [F966, F1001, F1002, F1007]
- `gfx_v1*_0_gpu_early_init` calls `BUG()` on unhandled `amdgpu_ip_version(adev, GC_HWIP, 0)`, so new GC 11.x/12.x IP versions must be added there as well as in `sw_init` and CG/PG switches. [F981]
- In `gfx_v1*_0_sw_init` and `gfx_v11_0_soft_reset`, the 3-level queue loops iterate `j` (queue) outside `k` (pipe), so pass `(i, k, j)` as `(me, pipe, queue)` and reset `ring_id = 0` before the compute ring loop. [F986]
- `gfx_v1*_ring_insert_nop` requires `num_nop >= 1`, must return early after writing `ring->funcs->nop` when `num_nop == 1`, and for `num_nop >= 2` emits `PACKET3(PACKET3_NOP, min(num_nop - 2, 0x3ffe))` then `amdgpu_ring_insert_nop(ring, num_nop - 1)`. [F989]
- RS64 PFP uses `regCP_GFX_RS64_DC_BASE0_LO`/`HI` and `PFP_PIPE*_RESET`, whereas RS64 ME uses `regCP_GFX_RS64_DC_BASE1_LO`/`HI` and `ME_PIPE*_RESET`. [F990]
- In `gfx_v1*_0_get_cu_info`, `cu_info->bitmap[0]` is `4x4` while `max_shader_engines` is up to 8 (2 SHs/SE): index via `cu_info->bitmap[0][i % 4][j + (i / 4) * 2]` and guard `disable_masks[i * 2 + j]` with `i < 8 && j < 2`. [F1004]
- In `gfx_v1*_0_update_coarse_grain_clock_gating`, only access `regSDMA1_RLC_CGCG_CTRL` when `adev->sdma.num_instances > 1`. [F1011]
