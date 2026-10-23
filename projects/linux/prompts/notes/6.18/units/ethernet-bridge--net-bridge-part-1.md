# ETHERNET BRIDGE — net/bridge (part 1)

This component implements core subsystems of the Linux Ethernet bridge, including IGMP/MLD multicast snooping and Explicit Host Tracking (EHT), Spanning Tree Protocol (STP) state machines and BPDU processing, bridge Netfilter (br_netfilter) IPv4/IPv6/ARP hook integration, and RTNetlink control plane configuration (including per-VLAN tunnel metadata). It manages multicast forwarding databases (MDB), querier/router state timers, STP port transitions with switchdev hardware offloading, and netlink link/AF operations.

## Architecture & Data Flow

- `br_multicast_new_port_group()` inserts `p->rhnode` into `br->sg_port_tbl` only when `!br_multicast_is_star_g(group)`, so both `br_multicast_del_pg()` and `br_multicast_del_port_group()` must guard `rhashtable_remove_fast(&br->sg_port_tbl, &pg->rhnode, br_sg_port_rht_params)` with `!br_multicast_is_star_g()`. [F5]
- A master `struct net_bridge_vlan` entry in `vg->vlan_list` without `BRIDGE_VLAN_INFO_BRENTRY` is a context-only placeholder; code iterating `vg->vlan_list` for active VLANs (such as netlink VLAN or tunnel dumps) must skip entries where `!br_vlan_should_use(v)`. [F46]
- In multicast snooping paths (`br_multicast.c`), `struct net_bridge_mcast_port *pmctx` is `NULL` for bridge/host-originated packets, so any function accepting a `pmctx` on receive, query, router, or leave paths must check `pmctx != NULL` before dereferencing `pmctx->port`. [F47]
- `pmctx->vlan` in `struct net_bridge_mcast_port` and `brmctx->vlan` in `struct net_bridge_mcast` are `NULL` for non-VLAN port/bridge multicast contexts and must only be dereferenced after `br_multicast_port_ctx_is_vlan(pmctx)` or `br_multicast_ctx_is_vlan(brmctx)` returns true. [F48]
- Stack `struct br_ip` and `struct net_bridge_port_group_sg_key` instances used as keys for `br->mdb_hash_tbl` or `br->sg_port_tbl` must be zeroed with `memset` before setting fields because rhashtable hashes the full struct size, including union tail bytes and padding. [F54]
- `br_netfilter_rtable_init()` must initialize `br->fake_rtable.dst` with the per-bridge writable `br->metrics` array via `dst_init_metrics(&rt->dst, br->metrics, false)` before calling `dst_metric_set(&rt->dst, RTAX_MTU, br->dev->mtu)`. [F63]
- Whenever `BROPT_MULTICAST_ENABLED` changes on a bridge after initialization—including automatic disablement in `br_multicast_new_group()` when `br->mdb_hash_tbl.nelems >= br->hash_max` and administrative toggling in `br_multicast_toggle()`—`br_mc_disabled_update()` must be called to synchronize `SWITCHDEV_ATTR_ID_BRIDGE_MC_DISABLED`. [F65]
- When `br_multicast_toggle_vlan()` updates multicast snooping across a VLAN's port entries, it must also check `br_vlan_is_brentry(vlan)` and call `br_multicast_toggle_one_vlan(vlan, on)` on the bridge master VLAN entry. [F66]
- When `br_nf_pre_routing_finish_bridge()` calls `neigh->output`, it must save the source MAC and EtherType in `nf_bridge->neigh_header` and set `nf_bridge->bridged_dnat = 1`, which `br_nf_pre_routing_finish_bridge_slow()` must restore while clearing `bridged_dnat = 0` and `physoutdev = NULL`. [F70]

## Synchronization

- `br_multicast_toggle()` must release `br->multicast_lock` before calling `br_multicast_join_snoopers(br)` or `br_multicast_leave_snoopers(br)`, and `br_ip4_multicast_join_snoopers()` / `br_ip4_multicast_leave_snoopers()` must balance `in_dev_get(br->dev)` with `in_dev_put(in_dev)`. [F18]
- In br_multicast_ctx_init(), brmctx->ip4_querier.seq and brmctx->ip6_querier.seq (declared as seqcount_spinlock_t) must be initialized with seqcount_spinlock_init() bound to &br->multicast_lock. [F20]
- All callers of br_ip4_multicast_mark_router(), br_ip6_multicast_mark_router(), br_ip4_multicast_add_router(), and br_ip6_multicast_add_router() (including br_multicast_pim(), br_ip4_multicast_mrd_rcv(), and br_ip6_multicast_mrd_rcv()) must hold br->multicast_lock. [F26]
- br_multicast_pg_to_port_ctx() and br_multicast_port_vid_to_port_ctx() must be called with br->multicast_lock held and must take rcu_read_lock() while looking up the VLAN and checking !br_multicast_port_ctx_vlan_disabled(&vlan->port_mcast_ctx). [F28]
- In br_ip4_multicast_igmp3_report() and br_ip6_multicast_mld2_report(), after calling br_ip4_multicast_add_group() or br_ip6_multicast_add_group() and re-acquiring br->multicast_lock, the code must revalidate br_multicast_ctx_should_use(brmctx, pmctx), re-lookup mdst and pg, and check !(pg->flags & MDB_PG_FLAGS_PERMANENT). [F29]
- In multicast expiration timer callbacks (such as br_multicast_group_expired(), br_multicast_port_group_expired(), and br_multicast_eht_set_expired()), after acquiring br->multicast_lock, the callback must verify the entry is still linked (not hlist_unhashed or RB_EMPTY_NODE) and !timer_pending() before acting on it. [F30]
- In br_nf_pre_routing_finish_bridge(), neighbour fields accessed without holding the neighbour lock (neigh->nud_state, neigh->hh.hh_len, and the function pointer neigh->output) must be loaded with READ_ONCE(). [F31]
- In br_message_age_timer_expired(), p->state == BR_STATE_DISABLED must be rechecked after acquiring spin_lock(&br->lock) despite the unlocked pre-check. [F36]

## Object Lifetimes

- When deinitializing a port or port-VLAN multicast context, `br_multicast_port_ctx_deinit()` must synchronously stop `ip4_mc_router_timer` and `ip6_mc_router_timer` and then unlink `pmctx` from both router lists via `br_ip4_multicast_rport_del()` and `br_ip6_multicast_rport_del()` under `br->multicast_lock`. [F1]
- `struct nf_bridge_info` records the ingress port by interface index (`physinif`) in `setup_pre_routing()`, and Netfilter resume callbacks (`br_nf_pre_routing_finish*`, `br_nf_forward_finish`) must resolve it via `nf_bridge_get_physindev()` and free `skb` if NULL. [F2]
- `ip_sabotage_in()` must mark `nf_bridge->sabotage_in_done = 1` rather than calling `nf_bridge_info_free(skb)`, preserving the `SKB_EXT_BRIDGE_NF` extension for downstream hooks until final bridge transmission in `br_nf_dev_queue_xmit()` or `br_nf_push_frag_xmit()`. [F3]
- Multicast entries removed under `br->multicast_lock` (`net_bridge_mdb_entry`, `net_bridge_port_group`, `net_bridge_group_src`, `net_bridge_group_eht_set`, `net_bridge_group_eht_set_entry`) must mark their list or RB-tree node unhashed (`hlist_del_init*` or `RB_CLEAR_NODE`) and defer `timer_shutdown_sync()` and freeing to `br->mcast_gc_list`. [F6]
- `br_multicast_del_port()` must flush `br->mcast_gc_work` after deleting all remaining port groups from `port->mglist` and before deinitializing `port->multicast_ctx` and freeing `port->mcast_stats`, and `br_multicast_dev_del()` must drain `mcast_gc_list`, cancel `mcast_gc_work`, and call `rcu_barrier()`. [F10]
- In `br_nf_pre_routing_finish_bridge()`, the neighbour reference obtained from `dst_neigh_lookup_skb(dst, skb)` must be released with `neigh_release(neigh)` on both the normal completion path and the early drop path when `nf_bridge_get_physindev(skb, net)` returns NULL. [F12]
- In `br_nf_local_in()`, if `ct_hook->confirm(skb)` returns a verdict whose `NF_VERDICT_MASK` is `NF_STOLEN`, ownership of `skb` has been consumed by `confirm` and `br_nf_local_in()` must return `NF_STOLEN` immediately without calling `nf_bridge_push_encap_header(skb)`. [F14]
- `br_stp_rcv()` takes ownership of the incoming `skb` from LLC and must release `br->lock` (if acquired) and free `skb` via `kfree_skb(skb)` across every normal and error exit path. [F19]

## API Contracts

- Callers of `br_multicast_list_adjacent()` must initialize `br_ip_list` before the call and free all `struct br_ip_list` entries linked into `br_ip_list` after the call (including when `kmalloc` fails mid-iteration and a partial count is returned). [F11]
- `br_multicast_new_group()` returns an `ERR_PTR` on failure (never `NULL`), whereas `br_multicast_new_port_group()` and `br_multicast_new_group_src()` return `NULL` on failure (never an `ERR_PTR`). [F41]
- `__br_multicast_add_group()` returns an `ERR_PTR` on allocation/limit failure, `NULL` when the context is disabled or when `!pmctx` (host-joined group), and a valid `struct net_bridge_port_group *` when a port group is found or created. [F42]
- `br_vlan_add()` and `nbp_vlan_add()` (including their `!CONFIG_BRIDGE_VLAN_FILTERING` stubs) must always initialize `*changed = false` before any return path because `br_vlan_info()` passes an uninitialized `bool curr_change` and reads it unconditionally even when an error is returned. [F56]
- `br_cfm_mep_count()` and `br_cfm_peer_mep_count()` (including their `!CONFIG_BRIDGE_CFM` stubs) must zero their `*count` out-parameter even when returning `-EOPNOTSUPP`, because `br_get_link_af_size_filtered()` uses the counts without checking the return value. [F57]
- `br_set_forward_delay()`, `br_set_hello_time()`, `br_set_max_age()`, `br_set_ageing_time()`, and `br_multicast_set_*_query_intvl()` take durations in `clock_t` (`USER_HZ`), whereas `__br_set_forward_delay()` and `__set_ageing_time()` take durations in `jiffies`. [F60]
- `br_multicast_has_querier_anywhere()`, `br_multicast_has_querier_adjacent()`, and `br_multicast_has_router_adjacent()` take `@proto` in host byte order (`ETH_P_IP` or `ETH_P_IPV6`), whereas `struct br_ip.proto` and `br_multicast_querier_exists()` use network byte order (`htons(ETH_P_*)`). [F61]
- Before `br_nf_forward_ip()` mutates per-egress fields in `struct nf_bridge_info` (such as `pkt_otherhost` or `physoutdev`), it must call `nf_bridge_unshare(skb)` and reload `nf_bridge = nf_bridge_info_get(skb)` so cloned multicast or broadcast SKBs do not overwrite a shared `SKB_EXT_BRIDGE_NF` extension. [F68]
- `br_multicast_ctx_should_use()` must evaluate `!br_multicast_port_ctx_state_disabled(pmctx)` when `pmctx` is non-NULL and only check `!br_multicast_ctx_vlan_disabled(brmctx)` when `pmctx` is NULL. [F105]

## Error Handling

- In `br_multicast_create_eht_set_entry()`, failure to create `eht_host` or `set_h` must only destroy `eht_host` if `hlist_empty(&eht_host->set_entries)` and only delete `eht_set` if `RB_EMPTY_ROOT(&eht_set->entry_tree)`, because `__eht_lookup_create_set()` and `__eht_lookup_create_host()` may return pre-existing populated objects. [F7]
- `br_multicast_port_ngroups_inc()` always increments `port->multicast_ctx.mdb_n_entries` first and, when `group->vid` has an enabled VLAN port context, also increments the VLAN context's `mdb_n_entries`, rolling back `port->multicast_ctx` if the VLAN quota check fails; `br_multicast_port_ngroups_dec()` must decrement both. [F8]
- `br_dev_newlink()` must roll back `register_netdevice(dev)` by calling `br_dev_delete(dev, NULL)` if `br_changelink()` fails, and `br_netlink_init()` must unwind `rtnl_af_register(&br_af_ops)` and `br_vlan_rtnl_init()` in reverse order on registration failure. [F17]
- When offloading bridge or port attributes via `switchdev_port_attr_set()` (including in `br_set_state()`, `__set_ageing_time()`, `br_multicast_add_port()`, and `br_multicast_toggle()`), `-EOPNOTSUPP` must be ignored and treated as success. [F43]
- In `br_multicast_ipv4_rcv()` and `br_multicast_ipv6_rcv()`, `-ENOMSG` from `ip_mc_check_igmp()` and `-ENOMSG` or `-ENODATA` from `ipv6_mc_check_mld()` indicate non-IGMP/MLD packets and must return `0` without calling `br_multicast_err_count()`, whereas other negative errnos must increment error stats and be returned. [F44]
- In `br_get_link_af_size_filtered()`, `dev` may be either a bridge port (`p != NULL`) or a bridge master (`br != NULL`), and `vg` may be `NULL`; port-only helpers (`br_mst_info_size(vg)` and `br_get_vlan_tunnel_info_size(vg)`) require `p != NULL` (and `vg != NULL` for `br_mst_info_size`), while CFM status sizing requires `br != NULL`. [F49]
- `br_nf_dev_queue_xmit()` must verify `skb_valid_dst(skb)` before invoking IPv4 or IPv6 fragmentation because tunnel ports can attach a `metadata_dst` whose `dst.dev` is `NULL`. [F50]
- In `br_setport()`, `p->flags` is mutated in place after saving `old_flags = p->flags`, and if either the `BR_PORT_MAB` prerequisite check (`BR_PORT_LOCKED` and `BR_LEARNING`) or `br_switchdev_set_port_flag()` fails, `p->flags` must be restored to `old_flags` before returning. [F82]

## External Contracts

- In br_setlink(), when IFLA_PROTINFO is present on a bridge port, the code must check protinfo->nla_type & NLA_F_NESTED and, if not nested, validate nla_len(protinfo) >= sizeof(u8) and treat the payload as a u8 port STP state via br_set_port_state() for binary compatibility with legacy RSTP. [F102]
- In `br_nf_local_in()`, packets marked with `BR_INPUT_SKB_CB(skb)->promisc` must have their conntrack state cleared via `nf_reset_ct(skb)` and return `NF_ACCEPT` immediately without attempting conntrack confirmation. [F151]
- In `br_fill_ifinfo()`, a single `IFLA_AF_SPEC` nested attribute must be opened once via `nla_nest_start_noflag(skb, IFLA_AF_SPEC)` before populating any `RTEXT_FILTER_*` sections and closed at `done:` with `nla_nest_end(skb, af)` if non-empty or cancelled with `nla_nest_cancel(skb, af)` if no child attributes were added. [F162]
- In `br_fill_linkxstats()`, the stack-allocated `struct bridge_vlan_xstats vxi` must be zero-initialized with `memset(&vxi, 0, sizeof(vxi))` before populating its fields and copying it to the netlink skb via `nla_put(skb, BRIDGE_XSTATS_VLAN, sizeof(vxi), &vxi)`. [F164]
- In `br_changelink()` and `br_setport()`, `IFLA_BR_GROUP_FWD_MASK` must reject any mask with bits in `BR_GROUPFWD_RESTRICTED` (`BR_GROUPFWD_STP | BR_GROUPFWD_MACPAUSE | BR_GROUPFWD_LACP`), whereas per-port `IFLA_BRPORT_GROUP_FWD_MASK` only rejects `BR_GROUPFWD_MACPAUSE`. [F166]

## Known Pitfalls

- `br_stp_set_bridge_priority()` must check `br_is_designated_port(p)` and update `p->designated_bridge.prio` before modifying `br->bridge_id.prio`, and `br_stp_change_bridge_id()` must save `oldaddr` before modifying `br->bridge_id.addr` so ports matching `oldaddr` can be updated. [F81]
- In br_stp_rcv(), after pulling the 3-byte LLC BPDU header prefix with skb_pull(skb, 3) and checking buf[0] == BPDU_TYPE_CONFIG, the code must call pskb_may_pull(skb, 32), reload buf = skb->data, and verify bpdu.message_age <= bpdu.max_age before calling br_received_config_bpdu(). [F85]
- In br_ip6_multicast_mld2_report(), before pulling a group record of variable size struct_size(grec, grec_src, nsrcs) with ipv6_mc_may_pull(), the code must verify skb_transport_offset(skb) + ipv6_transport_len(skb) >= nsrcs_offset + sizeof(__nsrcs) and read grec_nsrcs via skb_header_pointer(). [F87]
- MAC address buffers compared with ether_addr_equal() in bridge STP and multicast code (such as oldaddr_aligned and br_mac_zero_aligned in br_stp_if.c and net_bridge_port_group::eth_addr in br_private.h) must be at least 2-byte aligned. [F98]
- When `br_multicast_rcv()` resolves per-VLAN multicast contexts (`vlan != NULL` with `BROPT_MCAST_VLAN_SNOOPING_ENABLED`), it must branch on `br_vlan_is_master(vlan)` so `BR_VLFLAG_GLOBAL_MCAST_ENABLED` and `*brmctx` are read from `vlan` when master or from `vlan->brvlan` when a port VLAN. [F106]
- When iterating `mp->ports` via `struct net_bridge_port_group __rcu **pp` to delete multiple entries (as in `br_multicast_sg_del_exclude_ports()`), the loop must not advance `pp = &p->next` on iterations that call `br_multicast_del_pg(mp, p, pp)` because `br_multicast_del_pg()` already updates `*pp` to `p->next`. [F108]
- In `br_multicast_list_adjacent()`, because the local variable `port` is first used for `br_port_get_rcu(dev)` and then reused as the `list_for_each_entry_rcu(port, &br->port_list, list)` cursor, the loop body must exclude the querying port by comparing `port->dev == dev`. [F113]
- In `br_nf_hook_thresh()`, the loop finding the starting hook index `i` for `nf_hook_slow()` must skip hooks with `priority < NF_BR_PRI_BRNF`, break on `priority > NF_BR_PRI_BRNF`, and when `ops[i]->hook == br_nf_pre_routing` at `NF_BR_PRI_BRNF`, increment `i++` before breaking. [F118]

## Other

- When `br_process_vlan_info()` processes a VLAN range (`*vinfo_last != NULL`), it must track contiguous modified VIDs in `v_change_start`, emit `br_vlan_notify(br, p, v_change_start, v - 1, rtm_cmd)` and reset `v_change_start = 0` on any unmodified VID or after the loop, and clear `*vinfo_last = NULL` before returning. [F115]
