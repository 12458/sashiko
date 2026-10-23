# PIN CONTROLLER - QUALCOMM — drivers/pinctrl/qcom (part 1)

This unit contains the Qualcomm LPASS LPI (Low Power Island) pin controller core driver (`pinctrl-lpass-lpi.c`, `pinctrl-lpass-lpi.h`) and SoC-specific TLMM (Top-Level Mode Multiplexer) drivers for APQ8064, APQ8084, Glymur, IPQ4019, IPQ5018, IPQ5332, IPQ5424, IPQ6018, IPQ8064, IPQ8074, IPQ9574, MDM9607, MDM9615, and Milos. The LPASS LPI core implements pinctrl, pinmux, pinconf, and `gpio_chip` operations over MMIO registers and bulk clocks for audio island GPIOs, while the TLMM drivers define static pin/group/function tables and delegate runtime operation to `msm_pinctrl_probe()`.

## Architecture & Data Flow

- `lpi_pinctrl::slew_base` is only mapped in `lpi_pinctrl_probe()` when `LPI_FLAG_SLEW_RATE_SAME_REG` is not set in `lpi_pinctrl_variant_data::flags`; any code accessing slew-rate registers (such as `lpi_config_set_slew_rate()`) must check `LPI_FLAG_SLEW_RATE_SAME_REG` and use `tlmm_base` instead of dereferencing `slew_base`. [F10]
- When `lpi_config_set()` configures a pin as an output (`output_enabled` is true), it must write the output level (`LPI_GPIO_VALUE_OUT_MASK`) to `LPI_GPIO_VALUE_REG` before setting `LPI_GPIO_OE_MASK` in `LPI_GPIO_CFG_REG`. [F19]
- In `lpi_gpio_set_mux()`, `i == GPIO_FUNC && (val & LPI_GPIO_OE_MASK)` must precede `!test_and_set_bit(group, pctrl->ever_gpio)` in the short-circuit `&&` condition so non-GPIO or input-only muxing does not set `ever_gpio`, and on the first mux to `GPIO_FUNC` with `LPI_GPIO_OE_MASK` set, it must synchronize `LPI_GPIO_VALUE_OUT_MASK` to the sampled `LPI_GPIO_VALUE_IN_MASK` in `LPI_GPIO_VALUE_REG` before writing `LPI_GPIO_FUNCTION_MASK` to `LPI_GPIO_CFG_REG`. [F20, F38]
- In every SoC TLMM and LPASS LPI driver, a pin group name `"gpio<id>"` must be listed in `<fname>_groups[]` if and only if function `<fname>` is present in `PINGROUP(id, ...)` (or `LPI_PINGROUP(id, ...)` / `LPI_PINGROUP_OFFSET(id, ...)`). [F25]
- In `LPI_PINGROUP` and `LPI_PINGROUP_OFFSET` (`pinctrl-lpass-lpi.h`) and each SoC `PINGROUP` macro (e.g., `pinctrl-glymur.c`, `pinctrl-apq8064.c`), `.funcs` is initialized from an anonymous `(int[]){ ... }` compound literal that decays to `unsigned int *`, so `.nfuncs` must equal the exact element count of that compound literal. [F48]
- Because `lpi_gpio_set_mux()`, `lpi_config_set()`, `lpi_gpio_read()`, and `lpi_gpio_write()` pass `pin` and `group` selectors interchangeably to index `state->data->groups[]`, every `struct lpi_pinctrl_variant_data` must maintain 1-to-1 identity indexing where `pins[i].number == i` and `groups[i].pin == i`. [F43]
- In `drivers/pinctrl/qcom/pinctrl-lpass-lpi.c`, `MAX_NR_GPIO` (which sizes the `ever_gpio` bitmap in `struct lpi_pinctrl`) must be at least as large as `data->npins` for every registered `struct lpi_pinctrl_variant_data`, because `lpi_pinctrl_probe()` rejects `data->npins > MAX_NR_GPIO` with `-EINVAL`. [F29]
- In `struct lpi_pingroup`, `slew_offset` must be either `LPI_NO_SLEW` (`-1`) or a bit offset in `[0, 30]`, and `lpi_config_set_slew_rate()` must check `slew_offset == LPI_NO_SLEW` and reject `slew > LPI_SLEW_RATE_MAX` before shifting 32-bit unmasked `slew` and 2-bit `LPI_SLEW_RATE_MASK` by `slew_offset` for `iowrite32()`. [F45, F46]
- In `struct lpi_pinctrl`, `tlmm_base` and `slew_base` are `char __iomem *` (1-byte stride), so `lpi_gpio_read()`, `lpi_gpio_write()`, and `lpi_config_set_slew_rate()` must pass unscaled byte offsets (`pin_offset`, `LPI_TLMM_REG_OFFSET * pin`, and register offsets) when adding to those base pointers. [F50]

## Synchronization

- Every read-modify-write update to `LPI_GPIO_CFG_REG` or `LPI_SLEW_RATE_CTL_REG` (in `lpi_gpio_set_mux()`, `lpi_config_set_slew_rate()`, and `lpi_config_set()`) must be serialized under `pctrl->lock`; because `lpi_config_set_slew_rate()` locks `pctrl->lock` internally, `lpi_config_set()` must invoke `lpi_config_set_slew_rate()` before acquiring `pctrl->lock`. [F6, F60]
- In `lpi_gpio_set_mux()`, `pctrl->lock` must be held continuously across reading `LPI_GPIO_CFG_REG`, testing and setting `pctrl->ever_gpio`, syncing `LPI_GPIO_VALUE_REG` to match `LPI_GPIO_VALUE_IN_MASK`, and writing `LPI_GPIO_FUNCTION_MASK` to `LPI_GPIO_CFG_REG`. [F7]
- `lpi_gpio_set_mux()`, `lpi_config_set_slew_rate()`, `lpi_config_set()`, and the `gpio_chip` callbacks `lpi_gpio_direction_input()`, `lpi_gpio_direction_output()`, and `lpi_gpio_set()` acquire the sleeping `struct mutex lock` in `struct lpi_pinctrl` and must only be called from process context. [F66]

## Object Lifetimes

- In `lpi_pinctrl_probe()`, `platform_set_drvdata(pdev, pctrl)` must precede `devm_pinctrl_register()` and `devm_gpiochip_add_data()` (because `lpi_config_get()`/`lpi_config_set()` call `dev_get_drvdata(pctldev->dev)` and `lpi_pinctrl_remove()` calls `platform_get_drvdata(pdev)`), and `pctrl->ctrl = devm_pinctrl_register(...)` must complete before `lpi_build_pin_desc_groups()` and `devm_gpiochip_add_data()` (which pass `pctrl->ctrl` / `state->ctrl` to `pinctrl_generic_add_group()` and `lpi_config_set()`). [F1, F2]
- In `lpi_build_pin_desc_groups()`, `pinctrl_generic_add_group()` stores the `pins` pointer directly without copying, so `pins` must point to `&pin_info->number` inside the static `pctrl->desc.pins` array and `num_pins` must remain `1`. [F51]
- In `lpi_pinctrl_probe()`, bulk clocks `"core"` and `"audio"` enabled via `clk_bulk_prepare_enable()` and `pctrl->lock` initialized via `mutex_init()` are not devres-managed, so any failure after `mutex_init()` must jump to `err_pinctrl` and every platform driver using `lpi_pinctrl_probe` must pair it with `.remove = lpi_pinctrl_remove`. [F59]

## API Contracts

- Every `lpi_pingroup` definition (`LPI_PINGROUP` or `LPI_PINGROUP_OFFSET`) must place `LPI_MUX_gpio` at index `0` (`GPIO_FUNC`) of `lpi_pingroup::funcs`, because `lpi_gpio_set_mux()` writes the matched array index `i` directly into `LPI_GPIO_FUNCTION_MASK` and tests `i == GPIO_FUNC` for glitch-free GPIO output initialization. [F17]
- The `strength` variable in `lpi_config_set()` is expressed in milliamps (default `2` mA) and must be converted with `LPI_GPIO_DS_TO_VAL()` (`v / 2 - 1`) before writing `LPI_GPIO_OUT_STRENGTH_MASK`, whereas register reads must be converted back to milliamps with `lpi_regval_to_drive()` (`(val + 1) * 2`). [F16]
- In TLMM drivers defining `struct msm_pinctrl_soc_data`, all GPIO-capable pingroups (`PINGROUP` and `UFS_RESET`) must be placed at `groups[0 .. ngpios - 1]` indexed by GPIO number, while non-GPIO pingroups (`SDC_PINGROUP`, `SDC_QDSD_PINGROUP`) must be placed at indices `>= ngpios`. [F62]
- Any `reserved_gpios` array assigned to `struct msm_pinctrl_soc_data.reserved_gpios` (such as `ipq9574_reserved_gpios`) must be terminated by a negative sentinel (`-1`) and contain at most `ngpios` entries with every GPIO number `< ngpios`. [F64]

## Error Handling

- In `lpi_build_pin_desc_groups()`, `pinctrl_generic_add_group()` returns a non-negative group selector index on success and a negative errno on failure, so its return value must be checked with `ret < 0` (not `ret != 0`), and on failure at index `i` the rollback loop must iterate `for (; i > 0; i--)` removing group `i - 1`. [F12, F32]
- In `lpi_pinctrl_probe()`, `pctrl->clks[0].id` (`"core"`) and `pctrl->clks[1].id` (`"audio"`) must be initialized before calling `devm_clk_bulk_get_optional(dev, MAX_LPI_NUM_CLKS, pctrl->clks)` on the zero-allocated `struct lpi_pinctrl`. [F14]
- `lpi_gpio_dbg_show_one()` must normalize a `NULL` `pctldev` argument to `state->ctrl` (`pctldev = pctldev ? : state->ctrl;`) before dereferencing `pctldev->desc->pins[offset]`, because `lpi_gpio_dbg_show()` invokes `lpi_gpio_dbg_show_one()` with `pctldev = NULL`. [F15]

## Known Pitfalls

- In `lpi_config_set()`, local configuration variables (`pullup`, `strength`, and `output_enabled`) must be initialized to valid defaults before iterating over `configs[]` because all three fields are unconditionally written to `LPI_GPIO_CFG_REG`, and because `bool value` is declared uninitialized and written to `LPI_GPIO_VALUE_REG` whenever `output_enabled` is true, any `switch (param)` case that sets `output_enabled = true` must also assign `value`. [F9, F40]
