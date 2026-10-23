# PIN CONTROLLER - QUALCOMM — drivers/pinctrl/qcom (part 1)

This component contains the Qualcomm LPASS LPI (Low Power Island) pin controller core driver (`pinctrl-lpass-lpi.c` and `pinctrl-lpass-lpi.h`) and SoC-specific TLMM (Top-Level Mode Multiplexer) platform drivers for APQ8064, APQ8084, Glymur, IPQ4019, IPQ5018, IPQ5332, IPQ5424, IPQ6018, IPQ8064, IPQ8074, IPQ9574, MDM9607, MDM9615, and Milos. The LPASS LPI core implements pinctrl, pinmux, pinconf, and `gpio_chip` operations over MMIO registers and bulk clocks for audio island GPIOs, while the SoC TLMM drivers define static pin descriptors, mux function tables, pingroup layouts, reserved GPIOs, and PDC wakeup IRQ maps for `msm_pinctrl_probe()`.

## Architecture & Data Flow

- `lpi_pinctrl::slew_base` is only mapped in `lpi_pinctrl_probe()` when `LPI_FLAG_SLEW_RATE_SAME_REG` is not set in `lpi_pinctrl_variant_data::flags`. Any code accessing slew-rate registers (such as `lpi_config_set_slew_rate()`) must check `LPI_FLAG_SLEW_RATE_SAME_REG` and use `tlmm_base` instead of dereferencing `slew_base`. [F10]
- In `struct lpi_pinctrl`, `tlmm_base` and `slew_base` are `char __iomem *` (1-byte stride), so `lpi_gpio_read()`, `lpi_gpio_write()`, and `lpi_config_set_slew_rate()` must pass unscaled byte offsets (`pin_offset`, `LPI_TLMM_REG_OFFSET * pin`, and register offsets) when adding to those base pointers. Broken by: changing `tlmm_base` or `slew_base` to `u32 __iomem *` without adjusting offset arithmetic, or supplying word indices instead of byte offsets. [F50]
- `lpi_config_set_slew_rate()` must reject `slew > LPI_SLEW_RATE_MAX` before executing `sval |= slew << slew_offset`, because `slew` is not masked to `LPI_SLEW_RATE_MASK` when ORed into `sval`. Broken by: increasing `LPI_SLEW_RATE_MAX` without widening `LPI_SLEW_RATE_MASK`, or removing the bounds check. [F46]
- In `pinctrl-lpass-lpi.c`, `MAX_NR_GPIO` (which sizes the `ever_gpio` bitmap in `struct lpi_pinctrl`) must be at least as large as `data->npins` for every registered `struct lpi_pinctrl_variant_data`, because `lpi_pinctrl_probe()` rejects `data->npins > MAX_NR_GPIO` with `-EINVAL`. [F29]
- In every SoC TLMM and LPASS LPI driver, a pin group name `"gpio<id>"` must be listed in `<fname>_groups[]` if and only if function `<fname>` is present in `PINGROUP(id, ...)` (or `LPI_PINGROUP(id, ...)` / `LPI_PINGROUP_OFFSET(id, ...)`). [F25]
- In `LPI_PINGROUP` and `LPI_PINGROUP_OFFSET` (`pinctrl-lpass-lpi.h`) and each SoC `PINGROUP` macro (e.g., `pinctrl-glymur.c`, `pinctrl-apq8064.c`), `.funcs` is initialized from an anonymous `(int[]){ ... }` compound literal that decays to `unsigned int *`, so `.nfuncs` must equal the exact element count of that compound literal. [F48]

## Synchronization

- In `pinctrl-lpass-lpi.c`, every read-modify-write update to `LPI_GPIO_CFG_REG` or `LPI_SLEW_RATE_CTL_REG` (in `lpi_gpio_set_mux()`, `lpi_config_set_slew_rate()`, and `lpi_config_set()`) must be serialized under `pctrl->lock` because the pinctrl core does not serialize concurrent mux, config, and slew-rate callbacks. [F6]

## Object Lifetimes

- In `lpi_pinctrl_probe()`, `platform_set_drvdata(pdev, pctrl)` must be called before `devm_pinctrl_register()` and `devm_gpiochip_add_data()` because `lpi_config_get()` and `lpi_config_set()` retrieve `struct lpi_pinctrl *` via `dev_get_drvdata(pctldev->dev)` and `lpi_pinctrl_remove()` retrieves it via `platform_get_drvdata(pdev)`. [F1]
- In `lpi_pinctrl_probe()`, `pctrl->ctrl = devm_pinctrl_register(...)` must complete before `lpi_build_pin_desc_groups()` and `devm_gpiochip_add_data()`, because `lpi_build_pin_desc_groups()` passes `pctrl->ctrl` to `pinctrl_generic_add_group()` and the `gpio_chip` callbacks (`lpi_gpio_direction_input()`, `lpi_gpio_direction_output()`) pass `state->ctrl` to `lpi_config_set()` and dereference `pctldev->desc`. [F2]
- In `lpi_build_pin_desc_groups()`, `pinctrl_generic_add_group()` stores the `pins` pointer directly without copying, so `pins` must point to `&pin_info->number` inside the static `pctrl->desc.pins` array and `num_pins` must remain `1`. Broken by: passing the address of a stack-local pin index variable or passing `num_pins > 1`. [F51]

## API Contracts

- Every `lpi_pingroup` definition (via `LPI_PINGROUP` or `LPI_PINGROUP_OFFSET`) must place `LPI_MUX_gpio` at index `0` (`GPIO_FUNC`) of `lpi_pingroup::funcs`, and keep `funcs[]` ordered by hardware mux selector values, because `lpi_gpio_set_mux()` writes the matched array index `i` directly into `LPI_GPIO_FUNCTION_MASK` and tests `i == GPIO_FUNC` for glitch-free GPIO output initialization. [F17]
- The `strength` variable in `lpi_config_set()` is expressed in milliamps (default `2` mA) and must be converted with `LPI_GPIO_DS_TO_VAL()` (`v / 2 - 1`) before writing `LPI_GPIO_OUT_STRENGTH_MASK`, whereas register reads must be converted back to milliamps with `lpi_regval_to_drive()` (`(val + 1) * 2`). Broken by: initializing `strength` to `0` instead of `2`, or passing an already-encoded register value to `LPI_GPIO_DS_TO_VAL()`. [F16]
- In TLMM drivers defining `struct msm_pinctrl_soc_data`, all GPIO-capable pingroups (`PINGROUP` and `UFS_RESET`) must be placed at `groups[0 .. ngpios - 1]` indexed by GPIO number, while non-GPIO pingroups (`SDC_PINGROUP`, `SDC_QDSD_PINGROUP`) must be placed at indices `>= ngpios`. Broken by: setting `.ngpios = ARRAY_SIZE(<soc>_groups)` when `SDC_PINGROUP` or `SDC_QDSD_PINGROUP` entries are present, or placing a non-GPIO pingroup below `ngpios`. [F62]
- Any `reserved_gpios` array assigned to `struct msm_pinctrl_soc_data.reserved_gpios` (such as `ipq9574_reserved_gpios`) must be terminated by a negative sentinel (`-1`) and contain at most `ngpios` entries with every GPIO number `< ngpios`. [F64]

## Error Handling

- In `lpi_build_pin_desc_groups()`, the return value of `pinctrl_generic_add_group()` must be checked with `ret < 0` rather than `ret != 0` because it returns a non-negative group selector index on success. If it fails at index `i`, the rollback loop must iterate `for (; i > 0; i--)` and remove group `i - 1` so only the `0 .. i - 1` groups already added are removed without underflowing `0`. [F12, F32]
- `lpi_gpio_dbg_show_one()` must normalize a `NULL` `pctldev` argument to `state->ctrl` (`pctldev = pctldev ? : state->ctrl;`) before dereferencing `pctldev->desc->pins[offset]`, because `lpi_gpio_dbg_show()` invokes `lpi_gpio_dbg_show_one()` with `pctldev = NULL`. [F15]
