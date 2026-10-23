# AMD DISPLAY CORE — drivers/gpu/drm/amd/display/amdgpu_dm (part 1)

Core Display Manager (DM) glue layer bridging the Linux DRM/KMS atomic modesetting framework, AMDGPU base driver IP block lifecycle, and the OS-agnostic Display Core (DC) engine. It manages display hardware initialization/teardown, suspend/resume, DRM connectors/encoders/CRTCs/planes state translation and atomic commits, DMUB firmware loading and notifications, backlight control, and HDA audio component binding.

## Architecture & Data Flow

- Before downcasting a `struct drm_connector *` with `to_amdgpu_dm_connector()`, connector loops and atomic state callbacks must exclude `DRM_MODE_CONNECTOR_WRITEBACK` because `SIGNAL_TYPE_VIRTUAL` links are allocated as `struct amdgpu_dm_wb_connector` rather than `struct amdgpu_dm_connector`. [F70]
- Before calling `drm_atomic_helper_resume()` in `dm_destroy_cached_state()`, `dm->cached_state` must have every CRTC's `active_changed` and `color_mgmt_changed` set to true, freesync config reset, and duplicated `dc_stream_state` (`dm_new_crtc_state->stream`) and `dc_plane_state` (`dm_new_plane_state->dc_state`) pointers released and nulled. [F53]
- `dmub_aux_fused_io_callback()` and `execute_fused_io()` must verify `ddc_line < ARRAY_SIZE(adev->dm.fused_io)` before indexing `dm.fused_io[ddc_line]`, and `sizeof(struct dmub_cmd_fused_request)` must not exceed `sizeof(sync->reply_data)` (`0x40` bytes) in `fused_io_sync`. [F73]
- In `dce60_register_irq_handlers()`, `dce110_register_irq_handlers()`, and `dcn10_register_irq_handlers()`, `int_params.irq_source` must be validated against the inclusive `[FIRST, LAST]` enum range for its IRQ class before indexing `vblank_params`, `vupdate_params`, `pflip_params`, or `vline0_params` at `irq_source - FIRST`. Broken by: indexing directly by CRTC/pipe loop index or without subtracting the base `DC_IRQ_SOURCE_*` constant after validation. [F75]
- `fill_dc_dirty_rects()` must ensure the total number of dirty rectangles (including the 2 plane bounding-box rects added when `mpo_requested && bb_changed`) does not exceed `DC_MAX_DIRTY_RECTS`, falling back to a single full-frame dirty rect in `dirty_rects[0]` otherwise. [F79]

## Synchronization

- When `amdgpu_in_reset(adev)` is true, `dm_suspend()` acquires `dm->dc_lock` and returns with it held across the GPU reset until `dm_resume()` unlocks it after `dm_gpureset_commit_state()` and `amdgpu_dm_irq_resume_late()`, before calling `amdgpu_dm_backlight_set_level()`. Broken by: calling a helper that acquires `dm->dc_lock` (like `amdgpu_dm_backlight_set_level()`) in the reset branch of `dm_resume()` before `mutex_unlock(&dm->dc_lock)`. [F144]
- In `amdgpu_dm_fini()`, when `adev->dm.idle_workqueue->running` is true, `adev->dm.idle_workqueue->enable` must be set to false and `adev->dm.idle_workqueue->work` flushed via `flush_work()` before freeing `adev->dm.idle_workqueue`. [F20]

## Object Lifetimes

- In `dm_resume()`, `dm_dmub_hw_resume()` must precede powering on DMUB and DC (`dc_dmub_srv_set_power_state()` and `dc_set_power_state()`), `amdgpu_dm_irq_resume_early()` must precede `s3_handle_mst()` and `dm_destroy_cached_state()`, and `drm_dp_mst_topology_queue_probe()` and `amdgpu_dm_irq_resume_late()` must follow `dm_destroy_cached_state()`. [F136]

## API Contracts

- In `scale_input_to_fw()` and `scale_fw_to_input()`, custom brightness curve rescaling must use `AMDGPU_MAX_BL_LEVEL` (`0xFF`) to match the 8-bit `caps->luminance_data[].input_signal` table, whereas `amdgpu_dm_register_backlight_device()`'s fallback `props.max_brightness` must use `MAX_BACKLIGHT_LEVEL` (`0xFFFF`). [F39]

## Error Handling

- In `dm_cache_state()`, `adev->dm.cached_state` returned by `drm_atomic_helper_suspend()` must be checked with `IS_ERR()` and reset to `NULL` after extracting `PTR_ERR()`, because `dm_suspend()` and `dm_destroy_cached_state()` treat any non-NULL `cached_state` as a valid `drm_atomic_state` pointer. [F36]

## Known Pitfalls

- In `amdgpu_dm_commit_zero_streams()`, streams from `context->streams` must be copied into a separate `del_streams` array before calling `dc_state_remove_stream(dc, context, ...)` in a second loop, because `dc_state_remove_stream()` mutates `context->streams` and `context->stream_count` in place. [F89]
- `dm_free_gpu_mem()` and `dm_sw_fini()` iterate `adev->dm.da_list` with `list_for_each_entry` and must `break` immediately after calling `list_del(&da->list)` and `kfree(da)`, or use `list_for_each_entry_safe` if removing multiple `struct dal_allocation` entries. [F90]
- In `mmhub_read_system_context()`, on APUs with `AMD_APU_IS_RAVEN2 | AMD_APU_IS_RENOIR | AMD_APU_IS_GREEN_SARDINE`, `logical_addr_high` must add `0x1` to `adev->gmc.fb_end >> 18`, and the 32-bit shifted aperture values (`logical_addr_low`, `logical_addr_high`, `agp_base`, `agp_bot`, `agp_top`) must be cast to `uint64_t` before left-shifting by 18 or 24 into `pa_config->system_aperture` 64-bit address fields. [F102, F154]
