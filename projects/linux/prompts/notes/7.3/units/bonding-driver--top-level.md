# BONDING DRIVER — (top level)

This unit provides the core header definitions, UAPI structures, and sysfs management interfaces (`bonding_masters`, per-bond `bonding` group, and per-slave attributes) for the Linux Ethernet Channel Bonding driver, defining data structures and inline helpers for bonding master devices, enslaved interfaces, configuration options, IEEE 802.3ad (LACP) state machines, and Adaptive Load Balancing (ALB/TLB) hash tables.

## Architecture & Data Flow

- `bond_set_slave_state` and `bond_commit_link_state` must clear `should_notify` and `should_notify_link` when `notify` is `true` and toggle them when `notify` is `false` (never setting them to `1` unconditionally); `bond_slave_state_notify` and `bond_slave_link_notify` must reset them to `0` after calling `bond_lower_state_changed`. [F210]
- `slave_show()` in `slave_sysfs_ops` unconditionally downcasts `attr` via `to_slave_attr(attr)` and invokes `slave_attr->show(slave, buf)`, so every attribute registered on `slave->kobj` (including all entries in `slave_attrs[]`) must be embedded in a `struct slave_attribute` with a non-NULL `.show` callback. [F282]
- The sysfs attribute pointer arrays `per_bond_attrs` (used by `bonding_group`) and `slave_attrs` (passed to `sysfs_create_files` and `sysfs_remove_files`) must remain terminated with a `NULL` sentinel. [F253]
- `BOND_MAX_NS_TARGETS` must not exceed `BOND_MAX_ARP_TARGETS` because IPv6 NS target slots (`0 .. BOND_MAX_NS_TARGETS - 1`) index directly into `slave->target_last_arp_rx[BOND_MAX_ARP_TARGETS]`. [F257]
- `TLB_HASH_TABLE_SIZE` and `RLB_HASH_TABLE_SIZE` must each be at least 256 because `alb_bond_info.tx_hashtbl` and `alb_bond_info.rx_hashtbl` are indexed directly by the unmasked `u8` return value of `_simple_hash()`. [F260]
- `BOND_TLB_REBALANCE_INTERVAL` in `include/net/bond_alb.h` must remain strictly non-zero because TLB load calculations divide `entry->tx_bytes` and `bond_info->unbalanced_load` by `BOND_TLB_REBALANCE_INTERVAL`. [F273]
- In `include/net/bond_3ad.h`, `bond_3ad_churn_desc()` indexes `churn_description[]` directly by `churn_state_t` and clamps out-of-range values to `ARRAY_SIZE(churn_description) - 1` (`"unknown"`), so `churn_state_t` enumerators must stay 0-based and ordered identically to `churn_description[]` with `"unknown"` last. [F249]

## Synchronization

- Non-RCU helpers `bond_for_each_slave`, `bond_first_slave`, `bond_last_slave`, `bond_slave_has_mac`, and `bond_slave_get_rtnl` require `rtnl_lock()`; callers of `bond_for_each_slave_rcu`, `bond_first_slave_rcu`, `bond_slave_has_mac_rcu`, `bond_slave_get_rcu`, and `bond_option_active_slave_get_rcu` (which also accepts `rtnl_lock()`, as in `bonding_show_active_slave`) must hold `rcu_read_lock()` across both the call and all uses of the returned borrowed pointers (or pin `slave_dev` with `dev_hold` before unlocking). [F211, F212, F215, F216]
- `bond_set_active_slave`, `bond_set_backup_slave`, and `bond_set_slave_state` or `bond_commit_link_state` with `BOND_SLAVE_NOTIFY_NOW` require `rtnl_lock()` for `bond_lower_state_changed`; callers holding only `rcu_read_lock()` or `bond->mode_lock` without RTNL must pass `BOND_SLAVE_NOTIFY_LATER` and later call `bond_slave_state_notify` or `bond_slave_link_notify` under RTNL. [F217]
- In `bonding_store_bonds`, the `+` command path must call `bond_create` without holding `rtnl_lock`, whereas the `-` command path must hold a single `rtnl_lock()` critical section across both `bond_get_by_name` and `unregister_netdevice`; lockless per-netns `bn->dev_list` traversal (as in `bonding_show_bonds`) must hold `rcu_read_lock()` and use `list_for_each_entry_rcu`. [F204, F213]
- `__bond_3ad_get_active_agg_info` traverses the slave list via `bond_for_each_slave_rcu` without taking `rcu_read_lock()`, so callers not already in an RCU read-side critical section (such as `bonding_show_ad_aggregator` and other `bonding_show_ad_*` sysfs handlers) must call `bond_3ad_get_active_agg_info` instead. [F219]
- Because `slave->queue_id` and `slave->last_tx` are read locklessly under RCU in sysfs (`queue_id_show`), packet TX/RX, and link monitors while updated on other CPUs, concurrent accesses to `slave->queue_id` must use `READ_ONCE`/`WRITE_ONCE` and accesses to `slave->last_tx` must use `slave_last_tx` and `slave_update_last_tx`. [F220]
- `bond->slave_cnt` must only be modified in the slave attach/detach paths under `rtnl_lock`, and lockless readers (such as round-robin packet/XDP TX paths using it as a modulo divisor) must read it with `READ_ONCE(bond->slave_cnt)`. [F221]

## Object Lifetimes

- `bond_sysfs_slave_add` and `bond_sysfs_slave_del` must pass the identical `slave_attrs` array and `&slave->kobj` to `sysfs_create_files` and `sysfs_remove_files`, and during slave release or enslave rollback `bond_sysfs_slave_del` must synchronously remove `slave_attrs` before `bond_3ad_unbind_slave` clears 802.3ad state and before `kobject_put(&slave->kobj)` frees `SLAVE_AD_INFO(slave)` and `slave`. [F207, F223]
- `bonding_sysfs_store_option` retains ownership of the `buffer_clone` string allocated via `kstrndup` and must `kfree(buffer_clone)` after `bond_opt_tryset_rtnl` returns on both success and error paths. [F205]
- Per-bond sysfs attributes are registered by assigning `&bonding_group` to `bond->dev->sysfs_groups[0]` in `bond_prepare_sysfs_group` prior to netdevice registration; never manually call `sysfs_create_group` or `sysfs_remove_group` for `bonding_group` while `sysfs_groups[0]` is populated. [F289]
- `bond_tx_drop` increments `dev`'s core `tx_dropped` counter, frees `skb` via `dev_kfree_skb_any(skb)`, and returns `NET_XMIT_DROP`; callers must not free `skb` or increment `tx_dropped` again after calling `bond_tx_drop`. [F311]

## API Contracts

- `__bond_opt_init` reserves `optval->value = ULLONG_MAX` as the sentinel to select string/extra parsing over numeric parsing and only copies `extra` into `optval->extra` when `extra_len <= BOND_OPT_EXTRA_MAXLEN` (16 bytes); callers of `__bond_opt_init` and `bond_opt_initval` must pass either a valid `string` (with `value == ULLONG_MAX`) or a numeric `value` (never both), and must not allow a signed 32-bit `-1` / `0xffffffff` to sign-extend to `ULLONG_MAX` for non-`BOND_OPTFLAG_RAWVAL` numeric options. [F230, F277]
- Every `DEVICE_ATTR` in `drivers/net/bonding/bond_sysfs.c` using `bonding_sysfs_store_option` as its `.store` handler must have an `attr.name` identical to the `.name` of its entry in `bond_opts[]`, because `bonding_sysfs_store_option` resolves options via `bond_opt_get_by_name(attr->attr.name)`. [F239]
- `bond_option_active_slave_get_rcu(bond)` returns `NULL` when `!bond_uses_primary(bond)` (modes other than active-backup, TLB, and ALB) or when `bond->curr_active_slave` is `NULL`, so callers must check the returned `struct net_device *` for `NULL` before dereferencing. [F232]
- `bond_first_slave` and `bond_last_slave` guard `netdev_adjacent_get_private` with `bond_has_slaves(bond)` and return `NULL` when the slave list is empty; callers of `bond_first_slave`, `bond_last_slave`, and `bond_first_slave_rcu` must check for a `NULL` return. [F272]
- `bond_get_targets_ip` and `bond_get_targets_ip6` stop scanning at the first zero/unspecified address (unless searching for `0` or `in6addr_any` to find a free slot) and return `-1` when not found; the `targets` array must remain packed with no interior zero holes, and the return value must be stored in a signed `int` and checked for `-1` before indexing. [F233]
- `bond_netpoll_send_skb` in `include/net/bonding.h` calls `BUG()` when `CONFIG_NET_POLL_CONTROLLER` is disabled, so callers must only invoke it when netpoll is active on the slave. [F292]

## Error Handling

- `bond_3ad_get_active_agg_info` leaves its `struct ad_info *ad_info` out-parameter unpopulated when it returns non-zero (`< 0`), so callers (such as `bonding_show_ad_aggregator`, `bonding_show_ad_num_ports`, `bonding_show_ad_actor_key`, `bonding_show_ad_partner_key`, and `bonding_show_ad_partner_mac`) must check that it returns `0` before reading `ad_info`. [F229]

## External Contracts

- `bonding_store_bonds` and `bonding_sysfs_store_option` must return `count` on success or a negative errno on failure (never `0`, which causes userspace writes to retry forever); `bonding_store_bonds` must extract at most `IFNAMSIZ` bytes (`sscanf(buffer, "%16s", command)` into `char command[IFNAMSIZ + 1]`) starting with `'+'` or `'-'` followed by a name passing `dev_valid_name()`, and `bonding_sysfs_store_option` must bound the input string to `count` bytes via `kstrndup(buffer, count, GFP_KERNEL)` before calling `bond_opt_tryset_rtnl`. [F250, F251, F305]
- Wire-format structures `struct mac_addr`, `struct lacpdu`, `struct lacpdu_header`, `struct bond_marker`, and `struct bond_marker_header` in `include/net/bond_3ad.h` must remain byte-packed (`__packed` / `#pragma pack(1)`). [F304]

## Known Pitfalls

- In `struct bond_params`, `ad_actor_system` is declared as `u8 ad_actor_system[ETH_ALEN + 2]` (8 bytes) for `ether_addr_equal_64bits()` padding; always use `ETH_ALEN` (6) rather than `sizeof(bond->params.ad_actor_system)` when copying or serializing the MAC address. [F281]
- In `struct bond_marker`, `requester_port` (`u16`) and `requester_transaction_id` (`u32`) are typed as host integers rather than `__be16`/`__be32` despite holding big-endian wire values on receive, so any code reading or writing their numeric values must byte-swap them. [F264]
