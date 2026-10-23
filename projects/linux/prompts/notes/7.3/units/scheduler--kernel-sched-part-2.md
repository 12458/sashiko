# SCHEDULER — kernel/sched (part 2)

Implements the Deadline scheduling class (`SCHED_DEADLINE` using EDF, Constant Bandwidth Server, GRUB bandwidth reclaiming, and DL servers such as `fair_server`), the BPF extensible scheduler class (`sched_ext` / `SCHED_EXT`) with its built-in idle CPU selection policy, and the scheduler debugfs/procfs introspection and tuning interfaces.

## Architecture & Data Flow

- Standalone deadline servers (`dl_server(dl_se)` is true, e.g., `rq->fair_server`) are not embedded in `struct task_struct`. Shared `sched_dl_entity` paths must guard with `!dl_server(dl_se)` before calling `dl_task_of(dl_se)` or helpers like `update_stats_dequeue_dl()` that unconditionally downcast `dl_se`. [F143]
- When `update_curr_dl_se()` throttles a `dl_server` entity (`dl_server(dl_se)` is true) and `is_dl_boosted(dl_se)` holds or `start_dl_timer(dl_se)` returns `0`, it must call `replenish_dl_new_period(dl_se, rq)` and `start_dl_timer(dl_se)` rather than `enqueue_task_dl()` or `enqueue_dl_entity()`. [F67]
- In `replenish_dl_new_period()`, `replenish_dl_entity()`, and `dl_entity_overflow()`, static reservation parameters (`dl_runtime`, `dl_deadline`, `dl_period`) must be read from `pi_of(dl_se)`, whereas dynamic `runtime` and `deadline` fields are read and updated directly on `dl_se`. [F128]
- Adding or removing a task in `rq->dl.pushable_dl_tasks_root` via `enqueue_pushable_dl_task()` or `dequeue_pushable_dl_task()` must keep `rq->dl.earliest_dl.next` tracking the leftmost node's deadline and transition `rq->dl.overloaded`, `rq->rd->dlo_mask`, and `rq->rd->dlo_count` in lockstep via `dl_set_overload()` / `dl_clear_overload()`. [F69]
- When subtracting `dl_bw` from `dl_rq->running_bw` in `__sub_running_bw()` or from `dl_rq->this_bw` in `__sub_rq_bw()`, check whether the new `u64` value exceeds the previous value `old` and clamp it to `0` on underflow. [F134]
- In `sched_debug_sops` (`sched_debug_start()`, `sched_debug_next()`, `sched_debug_show()`), the `seq_file` iterator `v` encodes the header at `(void *)1` (`*offset == 0`) and online `cpu` at `(void *)(unsigned long)(cpu + 2)` (`*offset == cpu + 1`), advancing across sparse online CPUs via `cpumask_first()` / `cpumask_next()`. Broken by: treating `v` or `*offset` as a 0-based dense CPU index instead of decoding `(unsigned long)(v - 2)` and iterating `cpu_online_mask`. [F104]
- In the fallback branch of `SEQ_printf_task_group_path()`, `task_group_path(tg, buf, bufend - buf)` is bounded to `bufend - buf` (`sizeof(buf) - 3`) bytes so that `strcpy(bufend - 1, "...")` overwrites the final NUL byte only when the path filled `bufend - buf` bytes while staying within `buf[128]`. Broken by: passing `sizeof(buf)` to `task_group_path()` or writing `"..."` at `bufend`. [F153]

## Synchronization

- In `push_dl_task()` and `find_lock_later_rq()`, because `double_lock_balance(rq, later_rq)` may drop `rq->__lock`, callers must pin `next_task` with `get_task_struct(next_task)` before `find_lock_later_rq(next_task, rq)`, revalidate after locking that `!is_migration_disabled(task)`, `later_rq->cpu` is in `task->cpus_mask`, and `task` is still eligible on `rq` (`pick_next_pushable_dl_task(rq)`), and call `put_task_struct(next_task)` before reassigning `next_task = task` when retrying with a newly picked task. [F6, F22]
- Callers in `kernel/sched/deadline.c` holding a pinned `rq->__lock` (`__push_dl_task()`, `balance_dl()`, `dl_task_timer()`) must unpin it (`rq_unpin_lock(rq, rf)` or `lockdep_unpin_lock(__rq_lockp(rq), rf.cookie)`) before calling `push_dl_task()`, `pull_dl_task()`, or `dl_task_offline_migration()`, and repin it afterward. [F176]
- In `dl_set_overload()`, setting the CPU's bit in `rq->rd->dlo_mask` must be ordered by `smp_wmb()` before `atomic_inc(&rq->rd->dlo_count)`, paired with `smp_rmb()` after `dl_overloaded(this_rq)` in `pull_dl_task()`, so any CPU observing a non-zero overload count also observes the set bit in `dlo_mask`. [F20]
- In `pull_dl_task()`, when a pushable task on `src_rq` has `is_migration_disabled(p)`, `stop_one_cpu_nowait(src_rq->cpu, push_cpu_stop, push_task, &src_rq->push_work)` must only be called after `double_unlock_balance(this_rq, src_rq)` and `raw_spin_rq_unlock(this_rq)` with preemption disabled. [F177]
- Accessing a CPU's root-domain deadline bandwidth state via `dl_bw_of(cpu)`, `dl_bw_cpus(cpu)`, or `__dl_update(dl_b, bw)` requires holding an RCU-sched read section (`rcu_read_lock_sched()` or `rq->__lock` with IRQs disabled); mutating `dl_b->total_bw`, `dl_b->bw`, or `rq->dl.extra_bw` additionally requires holding `dl_b->lock`. [F23]
- In `sched_dl_global_validate()` and `sched_dl_do_global()`, deduplicating root domains across CPUs via `dl_bw_visited(cpu, cookie)` and `dl_cookie` requires holding `sched_domains_mutex` (in addition to `sched_rt_handler()`'s mutex) and `rcu_read_lock_sched()` around each `cpu_rq(cpu)->rd` access. [F24]
- In `sched_init_dl_servers()` and `sched_fair_server_write()`, configuring or updating `rq->fair_server` via `dl_server_apply_params()` and `setup_new_dl_entity()` requires holding `rq`'s lock (with `dl_b->lock` acquired inside `dl_server_apply_params()`) and calling `update_rq_clock(rq)` before comparing `rq_clock(rq)`. [F25]
- In `kernel/sched/debug.c`, modifying scheduler jump-label static keys via `sched_feat_set()` in `sched_feat_write()` requires holding `cpus_read_lock()` and `inode_lock(inode)`, and calling `update_sched_domain_debugfs()` in `sched_verbose_write()` requires acquiring `cpus_read_lock()` before `sched_domains_mutex`. [F26]

## Object Lifetimes

- When arming or canceling `dl_se->dl_timer` or `dl_se->inactive_timer`, `get_task_struct(dl_task_of(dl_se))` is taken only when `!dl_server(dl_se)` and `!hrtimer_is_queued(timer)`, and `put_task_struct(dl_task_of(dl_se))` on cancel (`cancel_dl_timer()`, `dl_rq_change_utilization()`) is called only when `hrtimer_try_to_cancel()` returns `1` and `!dl_server(dl_se)`. [F2]
- In `dl_task_timer()` and `inactive_task_timer()`, `put_task_struct(p)` must be called only after `task_rq_unlock(rq, p, &rf)`, and neither `p`, `dl_se`, nor the embedded `struct hrtimer` may be dereferenced afterward. [F3]
- When a `SCHED_DEADLINE` task leaves the deadline policy, `sched_dl_overflow()` must not subtract `p->dl.dl_bw` from `dl_b` (leaving `__dl_sub()` to `switched_from_dl()` at 0-lag time); if the task re-enters `SCHED_DEADLINE` while `hrtimer_active(&p->dl.inactive_timer)` is true, `sched_dl_overflow()` must subtract the old `p->dl.dl_bw` before adding `new_bw` via `__dl_add()`. [F5]

## API Contracts

- In `update_curr_dl_se()` and `dl_server_update_idle_time()`, runtime accounting for deadline server entities (`dl_server(dl_se)`) must subtract raw unscaled `delta_exec` rather than calling `dl_scaled_delta_exec()`, which scales execution time by CPU frequency and capacity. [F43]
- In `__checkparam_dl()`, `sched_attr` deadline parameters in nanoseconds must satisfy `attr->sched_runtime >= (1ULL << DL_SCALE)` and have bit 63 clear on both `attr->sched_deadline` and `attr->sched_period`, while `sysctl_sched_dl_period_min` and `sysctl_sched_dl_period_max` must be converted from microseconds by multiplying by `NSEC_PER_USEC`. [F46]

## Error Handling

- `start_dl_timer()` returns `0` when the replenishment activation time is already in the past (`ktime_us_delta(act, now) < 0`) and `1` when armed; callers throttling a deadline entity must check `!start_dl_timer(dl_se)` and immediately replenish or re-enqueue on `0`. [F44]
- To prevent division by zero, `task_non_contending()` must return early when `dl_se->dl_runtime == 0` before calling `div64_long((dl_se->runtime * dl_se->dl_period), dl_se->dl_runtime)`, and `__dl_server_attach_root()` must return early when `!dl_bw_cpus(cpu)` before calling `__dl_add()`. [F135]
- In `proc_sched_show_task()`, `avg_atom` and `avg_per_cpu` must be computed using `div64_ul()` and `div64_u64()` only after verifying that `nr_switches` and `p->se.nr_migrations` are non-zero (assigning `-1LL` otherwise), never with `/` on 64-bit operands. [F142]

## Known Pitfalls

- In `pull_dl_task()`, `dmin` must be preserved across iterations of `for_each_cpu(cpu, this_rq->rd->dlo_mask)` and updated to each pulled task's `p->dl.deadline`, whereas `push_task` must be reset to `NULL` before `double_lock_balance()` on every iteration. [F122]
- In `grub_reclaim()`, the active utilization bound must be tested by comparing `u_inact + rq->dl.extra_bw > rq->dl.max_bw - dl_se->dl_bw` before subtracting `u_inact` and `rq->dl.extra_bw` from `rq->dl.max_bw`, because all operands are unsigned `u64` and `u_inact + rq->dl.extra_bw` can exceed `rq->dl.max_bw`. [F133]
- In `nsec_high()` and `nsec_low()`, `nsec` must be cast to `long long` and negated when negative before calling `do_div(nsec, 1000000)` (with `nsec_high()` negating the quotient back), because `do_div()` performs unsigned 64-bit division and `SPLIT_NS()` formats signed `s64` values such as `-1`. [F141]
- Because `sched_domain->flags` and `sched_group->flags` (`m->private` in `register_sd()` / `sd_flags_fops`) are `int` rather than `unsigned long`, `sd_flags_show()` must load `unsigned long flags = *(unsigned int *)m->private` into a local variable before passing `&flags` to `for_each_set_bit(idx, &flags, __SD_FLAG_CNT)`. [F155]
