# NETWORKING [GENERAL] — include/net (part 1)

Core networking headers defining structures, inline helpers, and APIs for IPv4/IPv6 protocols, FIB routing and rules, flow dissection and hardware offload, GRO/GSO packet aggregation, Generic Netlink, fair queueing (FQ), and tunneling encapsulations.

## Architecture & Data Flow
- `inet_timewait_sock` must keep `__tw_common` first, `inet_sock` must keep `sk` and `pinet6` first, and `inet_connection_sock` must keep `icsk_inet` first to preserve cast layouts and `__inet_sk_copy_descendant()`. [F87]
- `inet_hashinfo` listening buckets (`lhash2`) must offset nulls markers by `LISTENING_NULLS_BASE` (`1U << 29`) because sockets can transition from `TCP_ESTABLISHED` to `TCP_LISTEN` without an RCU grace period. [F21]
- Updating an INET socket's IPv4 address via `sk_daddr_set()`, `sk_rcv_saddr_set()`, or `inet_reset_saddr()` must update the paired IPv6 address (`sk_v6_daddr`, `sk_v6_rcv_saddr`, `inet6_sk(sk)->saddr`) when `IS_ENABLED(CONFIG_IPV6)`, and changing a bound socket's receive address requires `inet_bhash2_update_saddr()`. [F78]
- `inet_csk_reqsk_queue_is_full()` must test `inet_csk_reqsk_queue_len(sk) > sk->sk_max_ack_backlog` (not `>=`) so backlog 0 still permits one connection to match `sk_acceptq_is_full()`. [F82]
- A `struct fib_nh_common *` (`res->nhc`) may only be downcast to `struct fib_nh *` after checking `nhc->nhc_family == AF_INET`, as IPv6 nexthops embed `nh_common` inside `struct fib6_nh`. [F45]
- When `flowi_l3mdev` in `struct flowi_common` is derived from `flowi_oif` rather than `flowi_iif`, `FLOWI_FLAG_L3MDEV_OIF` must be set in `flowic_flags` (`flowi_flags`) for FIB rule `iif_is_l3_master`/`oif_is_l3_master` matching. [F160]
- In `struct flow_keys`, `addrs` must remain last and `basic` (`FLOW_KEYS_HASH_START_FIELD`) `__aligned(SIPHASH_ALIGNMENT)`. In `flow_dissector_key_ipv4_addrs`, `flow_dissector_key_ipv6_addrs`, and `flowi4`, `src`/`saddr` must immediately precede `dst`/`daddr`; in `flow_dissector_key_eth_addrs`, `dst` must immediately precede `src`. [F83, F85, F126]
- In `include/net/fq_impl.h`, every `sk_buff` removal from `fq_flow::queue` must call `fq_adjust_removal()` or `__fq_adjust_removal()` under `fq->lock`; `fq->flows_bitmap` (`flow - fq->flows`) must only be touched when `flow != &tin->default_flow`. [F141, F183]
- In `fq_tin_dequeue()`, if `dequeue_func` returns `NULL` for a flow on `tin->new_flows` while `tin->old_flows` is non-empty, move it to `tin->old_flows` via `list_move_tail()`; when unlinking a flow from `flow->flowchain` in `fq_tin_dequeue()` or `fq_flow_reset()`, set `flow->tin = NULL`. `fq_flow_drop()` must use `do { ... } while (packets < pending)` so at least one packet is dropped when `skb_queue_len(&flow->queue) == 1`. [F61, F102, F103]
- Transient per-iteration GRO fields in `struct napi_gro_cb` must live inside `struct_group(zeroed, ...)`. Every GRO dispatch helper (`call_gro_receive()`, `call_gro_receive_sk()`, `indirect_call_gro_receive_inet()`) must gate on `gro_recursion_inc_test(skb)` and, at `GRO_RECURSION_LIMIT` (15), set `NAPI_GRO_CB(skb)->flush |= 1` and return `NULL`. [F62, F105]
- In `struct gro_node`, `gro->rx_list` and `gro->rx_count` must update in lockstep: `gro_normal_list()` must reset both `INIT_LIST_HEAD(&gro->rx_list)` and `gro->rx_count = 0`. [F65]
- Mutating IPv6 ECN bits (`IP6_ECN_set_ce()`, `IP6_ECN_set_ect1()`) must sample `*(__be32 *)iph` before writing and fold it into `skb->csum` when `skb->ip_summed == CHECKSUM_COMPLETE`, whereas IPv4 (`IP_ECN_set_ce()`, `IP_ECN_set_ect1()`) updates `iph->check` instead. [F69]
- In `struct ip_tunnel_info`, `key.u.ipv4` vs `u.ipv6` is selected by `ip_tunnel_info_af(info)` (`IP_TUNNEL_INFO_IPV6`), and `ip_tunnel_key_init()` for IPv4 must zero `IP_TUNNEL_KEY_IPV4_PAD` and trailing padding after `IP_TUNNEL_KEY_SIZE`. `ip_tunnel_info_opts_set()` must set `info->options_len = len` before copying into `info->options` (`__counted_by(options_len)`) and OR `flags` into `info->key.tun_flags`, keeping option bits in lockstep across `ip_tunnel_{set,clear,is}_options_present()`. [F48, F74]
- `lwtunnel_headroom(lwtstate, mtu)` returns 0 when `lwtstate->headroom >= mtu`; callers computing `mtu - lwtunnel_headroom(dst->lwtstate, mtu)` in `ip_dst_mtu_maybe_forward()`, `ip_skb_dst_mtu()`, `ip6_skb_dst_mtu()`, and `ip6_dst_mtu_maybe_forward()` must pass the clamped `mtu`. [F99]
- `__ipv6_prefix_equal64_half()` and 32-bit `ipv6_prefix_equal()` must guard the mask shift with `len != 0` (`pbi != 0`) and `len <= 64` (`prefixlen <= 128`). [F116]

## Synchronization
- Lockless readers of `sk->sk_state` must use `inet_sk_state_load()` (`smp_load_acquire()`), paired with `inet_sk_state_store()` (`smp_store_release()`). [F24]
- `__inet_inherit_port()` requires local BH disabled because it takes `inet_bind_hashbucket->lock` with bare `spin_lock()`. Calling `ip_select_ident()` or `ip_select_ident_segs()` on a connected TCP socket requires holding the socket lock (`sock_owned_by_me(sk)`). [F29, F148]
- On `struct fib6_info`, `fib6_add_gc_list()` and `fib6_remove_gc_list()` require `f6i->fib6_table->tb6_lock`, and `fib6_add_gc_list()` must re-verify `f6i->fib6_node != NULL` under `tb6_lock`. `RTF_EXPIRES` in `f6i->fib6_flags` and `f6i->expires` must be updated together via `fib6_set_expires()` / `fib6_clean_expires()` under `tb6_lock`. [F31, F76]
- `fib6_lookup()` and `fib6_table_lookup()` callers must hold `rcu_read_lock()` across the lookup and `fib6_select_path()`. `fib6_get_cookie_safe()` (`rt6_get_cookie()`) also requires `rcu_read_lock()`, `rcu_dereference(f6i->fib6_node)`, `READ_ONCE(fn->fn_sernum)`, and `smp_rmb()`. [F32, F150]
- Route helpers reading net_device or netns without a ref or RTNL (`ip_dst_mtu_maybe_forward()`, `ip6_dst_mtu_maybe_forward()`) must hold `rcu_read_lock()` and use `dst_dev_rcu()` and `dev_net_rcu()`. [F146]
- All queue, flow, and tin mutations in `include/net/fq_impl.h` (`fq_flow_dequeue()`, `fq_flow_drop()`, `fq_tin_dequeue()`, `fq_flow_classify()`, `fq_tin_enqueue()`, `fq_{flow,tin}_filter()`, `fq_{flow,tin}_reset()`, `fq_reset()`) require holding `fq->lock`. [F16]
- `genlmsg_multicast_allns()` requires holding either `rtnl_lock` or `rcu_read_lock()`. [F18]
- In `iptunnel_xmit_stats()`, per-CPU `dev->tstats` updates require `get_cpu_ptr()`/`put_cpu_ptr()` and `u64_stats_update_begin(&tstats->syncp)`/`u64_stats_update_end(&tstats->syncp)`. [F34]
- `lwtunnel_set_redirect()` must read `dst->output` and `dst->input` with `READ_ONCE()` and write them with `WRITE_ONCE()`. [F36]
- `txopt_get()` must dereference `np->opt` under `rcu_read_lock()` and check `refcount_inc_not_zero(&opt->refcnt)`, paired with `txopt_put()` freeing via `kfree_rcu(opt, rcu)`. [F158]
- In `struct kcm_sock`, `tx_wait` (under `mux->lock`) and `tx_wait_more` (under KCM socket lock) must remain separate `bool` fields, never adjacent bitfields. [F154]

## Object Lifetimes
- Before `fib6_info_release()` drops the last `f6i->fib6_ref` reference, `f6i->gc_link` must already be unhashed from `tb6_gc_hlist` via `fib6_remove_gc_list()` under `tb6_lock`. [F5]
- `__inet_lookup()`, `__inet6_lookup()`, `__inet_lookup_skb()`, and `__inet6_lookup_skb()` return un-refcounted listener sockets (`*refcounted = false`); callers must check `*refcounted` before `sock_put(sk)` or pin via `refcount_inc_not_zero(&sk->sk_refcnt)` before leaving RCU. [F23]
- In `struct inet_peer`, `rid` and `rcu` share a union, so `peer->rid` must never be accessed after `refcnt` reaches 0. [F28]
- `dst_default_metrics` is shared read-only: `ip_dst_init_metrics()`, `ip_fib_metrics_put()`, and `ip_dst_metrics_put()` must check `!= &dst_default_metrics` before setting `DST_METRICS_REFCOUNTED`, modifying `refcnt`, or calling `kfree()`. [F77]
- On `FLOW_BLOCK_UNBIND`, drivers must move `block_cb` onto `offload->cb_list` via `flow_block_cb_remove()` or `flow_indr_block_cb_remove()` instead of calling `list_del()` and `flow_block_cb_free()` directly. [F142]

## API Contracts
- `fib_lookup()` always sets `FIB_LOOKUP_NOREF`, returning an RCU-borrowed `res->fi` (`struct fib_info *`); do not call `fib_info_put(res->fi)` or use it past `rcu_read_unlock()` without `fib_info_hold()`. [F13]
- In `struct fib6_info`, trailing `fib6_nh[]` is allocated only when `!f6i->nh`; check `!f6i->nh` before dereferencing `f6i->fib6_nh`. [F128]
- Downcasting `struct fib_notifier_info *info` requires checking both `info->family` and `enum fib_event_type`. Mutating FIB rules via `struct fib_rules_ops` must invoke `ops->flush_cache` when provided. [F80, F139]
- `sk_to_full_sk()`, `sk_const_to_full_sk()`, and `skb_to_full_sk()` return `NULL` on `TCP_TIME_WAIT` even for non-NULL input; generic socket paths must check `inet_test_bit(IS_ICSK, sk)` before calling `inet_csk(sk)`. [F46]
- `inet_get_iif_sdif()` and `inet6_get_iif_sdif()` require `rcu_read_lock()`, `skb->dev != NULL`, and `!skb_valid_dst(skb)` (otherwise use `inet_iif()`/`inet_sdif()` or `inet6_iif()`/`inet6_sdif()`). [F169]
- `struct ipcm_cookie` and `struct ipcm6_cookie` use `-1` in `ipcm->tos` and `ipc6->hlimit` as unset sentinels; initialize via `ipcm_init()`, `ipcm_init_sk()`, or `ipcm6_init_sk()`, never `{ 0 }`. [F52]
- `ip6_flow_hdr()` and `ip6_make_flowinfo()` require `flowlabel` pre-masked with `IPV6_FLOWLABEL_MASK` (`flowi6_get_flowlabel()`). `fl6_merge_options(&opt_space, ...)` and `ipv6_fixup_options(&opt_space, ...)` may return `&opt_space` (stack-allocated), which must never escape the stack or be passed to `txopt_put()` or `ipv6_dup_options()`. [F59, F137]
- `ipv6_addr_prefix()` and `ipv6_addr_prefix_copy()` require `0 <= plen <= 128`. `__ipv6_addr_diff()` and `__ipv6_addr_diff32()` require `addrlen` divisible by 4, and `__ipv6_addr_diff64()` requires `!(addrlen & 7)`. [F94, F118]
- When detaching packets from an `skb->next` chain in `fq_tin_enqueue()`, `ip_fraglist_next()`, or `ip6_fraglist_next()`, advance past `skb->next` and call `skb_mark_not_on_list(skb)` first. [F104]
- Check `flow_rule_match_key(rule, FLOW_DISSECTOR_KEY_*)` before calling `flow_rule_match_*()`, and check `flow_action_has_entries(action)` (or `flow_offload_has_one_action(action)`) before `flow_action_first_entry_get(action)`. [F43, F44]
- `flow_action_hw_stats_check()` takes an unshifted `enum flow_action_hw_stats_bit`, whereas `flow_stats_update()` takes a single-bit shifted `enum flow_action_hw_stats` mask (never `FLOW_ACTION_HW_STATS_ANY`). In `__flow_action_hw_stats_check()`, `check_allow_bit == false` requires both bits of `FLOW_ACTION_HW_STATS_ANY` (`~action_entry->hw_stats & FLOW_ACTION_HW_STATS_ANY`). [F54, F108]
- Initialize `struct gnet_stats_basic_sync` with `gnet_stats_basic_sync_init()` and use `_bstats_set()`/`_bstats_update()` and `gnet_stats_add_basic()` whenever concurrent access is possible. [F17]
- In Generic Netlink: `genlmsg_multicast*()`, `genl_set_err()`, and `genl_has_listeners()` take a 0-based `group` index (`< family->n_mcgrps`), and `genl_set_err()` requires a negative errno. `family->split_ops` entries must set either `GENL_CMD_CAP_DO` or `GENL_CMD_CAP_DUMP`, sorted by `cmd` with `DO` before `DUMP`. Ops with `cmd >= family->resv_start_op` must have `validate == 0` and a `policy` if taking attributes. `GENL_ADMIN_PERM` checks `init_user_ns`, whereas `GENL_UNS_ADMIN_PERM` checks `net->user_ns`. [F56, F161, F162, F165]
- `skb_gro_header()`, `skb_gro_header_slow()`, and `skb_gro_remcsum_process()` may call `pskb_may_pull()`, invalidating prior header pointers and `frag0`; never call `skb_gro_header_fast()` when `!skb_gro_may_pull(skb, hlen)`. If `skb_gro_remcsum_process()` ran and GRO flushes without consuming the packet, use `skb_gro_flush_final_remcsum()` instead of `skb_gro_flush_final()`. [F64, F131]
- GSO head expansion must use `gso_pskb_expand_head()` to keep `SKB_GSO_CB(skb)->mac_offset` synced. `gso_reset_checksum()` must not overwrite `csum`/`csum_start` when `skb->remcsum_offload` is set, and `gso_make_checksum()` must sample `csum`/`csum_start` before updating them. [F66, F67]
- `INET_ECN_decapsulate()`, `IP_ECN_decapsulate()`, and `IP6_ECN_decapsulate()` return `0` (ok), `1` (log anomaly, do not drop), or `2` (drop; check `> 1`). Callers of `__INET_ECN_decapsulate()` must initialize `bool set_ce = false`. [F39, F110]
- `iptunnel_pull_offloads()` must succeed in `skb_unclone(skb, GFP_ATOMIC)` when `skb_is_gso(skb)` before clearing `NETIF_F_GSO_ENCAP_ALL` and `skb->encapsulation`. `iptunnel_xmit_stats()` treats `pkt_len > 0` as tx ok, `0` as `tx_dropped`, and `< 0` as `tx_errors`. [F41, F72]
- Tunnel flag bitmaps passed to `ip_tunnel_flags_*()`, `ip_tunnel_*_options_present()`, `ip_tunnel_key_init()`, `gre_flags_to_tnl_flags()`, and `gre_build_header()` must use `IP_TUNNEL_DECLARE_FLAGS()`. `gre_build_header()` requires `hdr_len == gre_calc_hlen(flags)`. [F91, F140]

## Error Handling
- `fl6_sock_lookup()` returns `ERR_PTR(-ENOENT)` when exclusive flow labels are active and `NULL` otherwise; always check `IS_ERR(fl)`. [F37]
- `skb_get_dsfield()` returns `0..255` or `-1` on error; store the result in a signed `int`, never `u8`. [F40]
- `ip6_route_output*()`, `ip6_route_lookup()`, and `fib6_rule_lookup()` return a valid `dst` with negative `dst->error` on failure (never `NULL`/`ERR_PTR`), whereas `ip6_dst_lookup_flow()` and `ip6_sk_dst_lookup_flow()` return `ERR_PTR(err)`. [F42]
- `inet_steal_sock()` and `inet6_steal_sock()` can return an `ERR_PTR` from reuseport lookup; check `IS_ERR(sk)` before `if (sk)`. [F111]

## External Contracts
- `enum flow_dissector_ctrl_flags` must keep `FLOW_DIS_*` synced with `TCA_FLOWER_KEY_FLAGS_*` and place internal flags at `(TCA_FLOWER_KEY_FLAGS_MAX << 1)`. [F100]
- Only append new `FIB_MULTIPATH_HASH_FIELD_*` bits, and mask `fl4->flowi4_dscp` with `INET_DSCP_LEGACY_TOS_MASK` (`0x1c`) in `fib_dscp_masked_match()`. [F101]

## Known Pitfalls
- Pass the user-header pointer from `genlmsg_put*()` (not `nlmsghdr *` or `genlmsghdr *`) to `genlmsg_end()`, `genlmsg_cancel()`, `genlmsg_nlhdr()`, and `genl_dump_check_consistent()`. When `genl_info_is_ntf(info)` (`genl_info_init_ntf()`), `info->genlhdr` aliases `&info->user_ptr[0]`/`info->ctx` and `info->nlhdr` is `NULL`; do not touch `ctx`/`user_ptr[0]` or call `genlmsg_len(info->genlhdr)`. [F55, F138]
- `__inet6_lookup()` takes host-order `u16 hnum`, whereas `__inet_lookup()` takes network-order `__be16 dport`. [F57]
- Use `key32_to_tunnel_id()` / `tunnel_id_to_key32()` for `__be32` <-> `__be64` tunnel keys, and cast 32-bit operands to `u64` before `<< 32` in `fib_multipath_hash_construct_key()` and `INET_ADDR_COOKIE()`. [F89, F124]
- Use `1ULL << key_id` (`BIT_ULL`) on `flow_dissector->used_keys` (`FLOW_DISSECTOR_KEY_IPSEC` is 32), and widen `icsk_rto` / `req->timeout` to `u64` before shifting in `inet_csk_rto_backoff()` and `reqsk_timeout()`. [F114, F115]
- In `inet_gro_flush()`, track outer/inner fixed-ID in 2-bit `ip_fixedid` via `1 << inner`, and truncate `(id >> 16) - (id2 >> 16)` to `u16` (`ipid_offset`). [F106, F119]
- Never dereference `__act` after `flow_action_for_each()` completes without breaking (`&entries[num_entries]`). [F107]
- `ip_decrease_ttl()` must widen `iph->check` to `u32` and fold carry via `check + (check >= 0xFFFF)`. [F120]
- Gate route/PMTU expiry in `fib6_check_expired()` and `ip_dst_mtu_maybe_forward()` on `RTF_EXPIRES` / `rt->rt_pmtu != 0` and use `time_after()`/`time_before()`. [F122]
- In `snmp_get_cpu_field*()`, `offt` is an array index while `syncp_offset` is a byte offset. [F130]
