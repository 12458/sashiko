# COMMON INTERNET FILE SYSTEM CLIENT (CIFS and SMB3) — fs/smb/client (part 3)

This unit implements the Linux VFS and netfs integration layer for the SMB client, including file, inode, dentry, and address_space operations, mount parameter parsing (`fs_context`), symlink/hardlink handling, ioctls, DFS automounts, and fscache integration.

## Architecture & Data Flow

- Under `cinode->open_file_lock`, `cifs_new_fileinfo()` must insert `FMODE_READ` `cifsFileInfo` entries at the head of `cinode->openFileList` (`list_add`) and write-only entries at the tail (`list_add_tail`), because `find_readable_file()` stops at the first non-`FMODE_READ` entry. [F205]
- Whenever an open, reopen, or oplock downgrade updates an inode's oplock (`cifs_new_fileinfo()`, `cifs_reopen_file()`, `cifs_oplock_break()`), a resulting read-only oplock must be reset to 0 if `cifs_has_mand_locks(cinode)` is true. [F254]
- In `cifs_get_disposition()` and `cifs_do_create()`, `(O_CREAT | O_EXCL)` and `(O_CREAT | O_TRUNC)` must be tested with full-mask equality before single-bit `O_CREAT` or `O_TRUNC` checks so `O_CREAT` does not shadow `FILE_CREATE` or `FILE_OVERWRITE_IF`. [F278]
- When `cifs_do_create()` finds a valid parent `cached_fid` (`is_valid_cached_dir(parent_cfid)`) for `direntry->d_parent`, it must copy `parent_cfid->fid.lease_key` to `fid->parent_lease_key`, set `SMB2_LEASE_FLAG_PARENT_LEASE_KEY_SET_LE`, and set `parent_cfid->dirents.is_valid = false` and `is_failed = true`. [F240]
- `struct netfs_io_request rreq` must remain at offset 0 of `struct cifs_io_request` (unioned via `cifs_io_subrequest`); `cifs_prepare_read()` and `cifs_prepare_write()` must only call `cifs_negotiate_rsize()`/`wsize()` when `cifs_sb->ctx->rsize`/`wsize` is 0. [F241, F305]
- `cifs_get_inode_info()` and `smb311_posix_get_inode_info()` must only short-circuit on `is_inode_cache_good(*inode)` when caller-supplied `@data` is `NULL`. [F242]
- `struct cifs_fscache_inode_key` must remain `__packed` and its fields (`uniqueid`, `createtime`, `type`) must match `cifs_find_inode()`, because `cifs_fscache_get_inode_cookie()` hashes `&key` (`sizeof(key)`) without zeroing padding. [F301]
- Jiffies-based cache expiration checks in `cifs_d_revalidate()` (`cifs_get_time(direntry)`) and `cifs_inode_needs_reval()` (`cifs_i->time`, unless 0) must use `time_after()`/`time_in_range()`, and `cifs_d_revalidate()` on a positive `IS_AUTOMOUNT(inode)` dentry must set `DCACHE_NEED_AUTOMOUNT` under `d_lock`. [F257, F297]
- In `cifs_do_automount()`, `source`, `leaf_fullpath`, `UNC`, `prepath`, and `dfs_root_ses` must be cleared on the temporary context before `smb3_fs_context_dup()`, and `fs_context_set_ids()` plus `is_dfs_mount()` must refresh the submount's IDs and `dfs_automount`/`dfs_conn` flags. [F245]
- In `smb3_fs_context_parse_param()` and `cifs_parse_cache_flavor()`, coupled options in `struct smb3_fs_context` must update in lockstep (`remap` vs `sfu_remap`, `direct_io`/`strict_io`/`cache_ro`/`cache_rw`, `multichannel` vs `max_channels`), and seconds-based timeouts (`Opt_acregmax`, `Opt_acdirmax`, `Opt_actimeo`, `Opt_closetimeo`, `Opt_echo_interval`) must be range-checked against `CIFS_MAX_ACTIMEO / HZ`, `SMB3_MAX_DCLOSETIMEO / HZ`, or `[SMB_ECHO_INTERVAL_MIN, SMB_ECHO_INTERVAL_MAX]` before multiplying by `HZ`. [F260, F265]
- In `struct smb3_fs_context`, `target_rfc1001_name[RFC1001_NAME_LEN_WITH_NULL]` must be space-padded (`0x20`) across all 16 bytes with at most 15 (`RFC1001_NAME_LEN`) hostname bytes copied so byte 15 stays `0x20`, whereas `source_rfc1001_name` pads 15 bytes with `0x20` and keeps byte 15 as `0x00`. [F311]

## Synchronization

- `_cifsFileInfo_put()`, `cifsFileInfo_put()`, and `cifs_reopen_file()` must never be called under `tcon->open_file_lock`, `cinode->open_file_lock`, or `cifs_file->file_info_lock`; callers iterating `openFileList` (`cifs_get_writable_file()`, `cifs_close_*deferred_file*()`, `cifs_reopen_persistent_handles()`) must pin entries with `cifsFileInfo_get()` or move them to a local list under `open_file_lock` and process with `list_for_each_entry_safe()` after dropping the lock, passing `wait_oplock_handler = false` from `cifs_oplock_break()` and `cifs_close_deferred_file()`. [F189, F222, F283]
- In `cifs_reopen_file()`, checking `cfile->invalidHandle`, reopening, and clearing `invalidHandle` require `cfile->fh_mutex`, which must be dropped before `filemap_write_and_wait()` or `cifs_relock_file()`; writeback/write-prep callers (`cifs_prepare_write()`, `cifs_get_writable_file()`, `cifs_reopen_persistent_handles()`) must pass `can_flush = false`. [F217, F237]
- In `cifs_writev()` and `cifs_strict_readv()`, `netfs_start_io_{write,direct,read}()` (`inode->i_rwsem`) must wrap `down_read(&cinode->lock_sem)`; write-locking `lock_sem` must use `cifs_down_write(&cinode->lock_sem)`, and `cifs_relock_file()` must use `down_read_nested(&cinode->lock_sem, SINGLE_DEPTH_NESTING)`. [F318, F319]
- `cifs_lookup()`, `cifs_reopen_file()`, and `__cifs_unlink()` must never acquire `sb->s_vfs_rename_mutex`. [F314]
- In `__cifs_unlink()` on SMB2+ (`protocol_id > SMB10_PROT_ID`), silly-rename (`-EBUSY`) must be checked after `cifs_close_deferred_file_under_dentry(tcon, dentry)` by testing `d_count(dentry) > 1` under `dentry->d_lock`. [F213]
- Before duplicating or reconfiguring `smb3_fs_context` in `cifs_do_automount()` or `smb3_reconfigure()`, callers must hold `ses->session_mutex` and call `smb3_sync_session_ctx_passwords()`. [F243]
- `cifs_release_automount_timer()` must only be called after `list_empty(&cifs_automount_list)`, because `cifs_expire_automounts()` re-arms `cifs_automount_task` while the list is non-empty. [F223]

## Object Lifetimes

- `cifs_add_deferred_close()` requires `cinode->deferred_lock` and takes ownership of `@dclose` (freeing on duplicate or linking onto `cinode->deferred_closes` for `cifs_del_deferred_close()`); stack-allocated `struct cifs_pending_open` linked via `cifs_add_pending_open*()` must be unlinked before return by `cifs_new_fileinfo()` or `cifs_del_pending_open()`. [F191, F307]
- Every successful VFS open (`cifs_open()` including `use_cache`, and `cifs_atomic_open()`) must call `fscache_use_cookie(..., file->f_mode & FMODE_WRITE)` paired with `cifs_fscache_unuse_inode_cookie()` in `cifs_close()`; in `cifs_do_create()` and `cifs_nt_open()`, any error after `server->ops->open()` or `cifs_posix_open()` (`cifs_get_inode_info*()` failure or `S_ISDIR(newinode->i_mode)`) must close the server handle and `iput(newinode)`. [F193, F198]
- Every `cifs_get_writer(cinode)` must pair with `cifs_put_writer(cinode)`; if `CIFS_INODE_PENDING_OPLOCK_BREAK` is seen after incrementing `cinode->writers`, `cifs_get_writer()` must decrement `writers` and clear/wake `CIFS_INODE_PENDING_WRITERS` when `writers == 0` before retrying. [F200]
- In `cifs_prepare_read()` and `cifs_prepare_write()`, `get_xid()` must only run when `!have_xid` (setting `have_xid = true`), and any unconsumed `credits.value` must be returned via `add_credits_and_wake_if()` on issue failure or in `cifs_free_subrequest()`. [F194]
- In `cifs_unlock_range()`, `struct cifsLockInfo` entries moved to `tmp_llist` for batched `cifs_lockv()` must be freed via `cifs_free_llist(&tmp_llist)` on success or restored via `cifs_move_llist(&tmp_llist, &cfile->llist->locks)` on failure under `cinode->lock_sem`. [F203]
- `smb311_posix_info_to_fattr()` and `cifs_open_info_to_fattr()` move `data->symlink_target` to `fattr->cf_symlink_target` (nulling `data->symlink_target`), and `cifs_fattr_to_inode()` moves `fattr->cf_symlink_target` to `cifs_i->symlink_target` (after `kfree(cifs_i->symlink_target)`, nulling `cf_symlink_target`). [F195]
- In `cifs_dump_full_key()`, when `out.session_id != 0`, lookup on `cifs_tcp_ses_list` must check `ses_it->ses_status != SES_EXITING` and call `cifs_smb_ses_inc_refcount(ses)` under `cifs_tcp_ses_lock` and `ses_it->ses_lock` (`found = true`); `cifs_put_smb_ses(ses)` must be called on exit iff `found` is true (never when borrowed from `tcon->ses`). [F199, F221]
- In `cifs_ioctl_query_info()`, when `path[0]` is NUL, `utf16_path` points to stack `root_path`; `ops->ioctl_query_info` must not free it and `ici_exit` must guard `kfree(utf16_path)` with `utf16_path != &root_path`. [F308]

## API Contracts

- In `cifs_issue_read()`, `NETFS_SREQ_CLEAR_TAIL` must only be set when `subreq->rreq->origin` is neither `NETFS_UNBUFFERED_READ` nor `NETFS_DIO_READ`; in `cifs_set_ops()`, regular files where `max_read < PAGE_SIZE + MAX_CIFS_HDR_SIZE` must use `cifs_addr_ops_smallbuf` (no `.readahead`). [F279, F335]
- When calling `SMB2_write()` (e.g. `smb3_create_mf_symlink()`), `iov[0]` is reserved for the SMB2 header; callers must place payload buffers starting at `iov[1]` and pass payload vector count `n_vec` excluding `iov[0]`. [F239]
- `build_path_from_dentry*()` and `automount_fullpath()` build backward in `page` from `dentry_path_raw(direntry, page, PATH_MAX)`; they must advance `s` by 1 when `!s[1]` (`"/"`), verify `s >= (char *)page + prefix_len` before prepending, and callers must pass the original `alloc_dentry_path()` buffer (never the returned interior pointer/`ERR_PTR`) to `free_dentry_path()`. [F263, F298]
- `cifs_convert_address()` returns `1` on success and `0` on failure (callers must map `!cifs_convert_address(...)` to a negative errno); callers of `cifs_convert_address()`, `dns_resolve_name()`, `resolve_name()`, and `dns_resolve_unc()` must pass a `struct sockaddr *` backed by at least `sizeof(struct sockaddr_in6)` (e.g. `struct sockaddr_storage`) and bound all operations on non-NUL-terminated `name`/`src` by `namelen`/`len` (`memchr`, `%.*s`). [F227, F309, F310]

## Error Handling

- `cifs_sanitize_prepath()` modifies `prepath` in-place without freeing it and returns a `kstrdup()` buffer (`NULL` if empty, `ERR_PTR(-ENOMEM)` on failure); callers storing `cifs_sanitize_prepath()` or `smb3_fs_context_fullpath()` into `ctx->prepath`, `cifs_sb->prepath`, or `ctx->source` must reset the field to `NULL` on `IS_ERR()` before cleanup calls `kfree()`. [F187, F225]
- In `cifs_ioctl()`, `filep->private_data` (`pSMBFile`) can be `NULL` (e.g. open directories), so handlers using `pSMBFile` must check `if (pSMBFile == NULL) break;` before dereferencing it. [F231]
- Callers of `parse_dfs_referrals()` must initialize `*target_nodes = NULL` before the call because early error paths pass `*target_nodes` to `free_dfs_info_array()`. [F233]

## External Contracts

- In `cifs_d_revalidate()`, `flags & LOOKUP_RCU` must immediately return `-ECHILD` before sleeping or taking blocking locks, and `-ENOENT`/`-ESTALE` from `cifs_revalidate_dentry()` must be translated to `0` rather than returned as negative errnos. [F234, F313]
- In `cifs_setattr()`, `ATTR_OPEN` must return 0 immediately without updating server timestamps, and `cifs_setattr_unix()`/`nounix()` must clear `ATTR_CTIME | ATTR_MTIME` after handling `ATTR_SIZE`. [F343]
- `smb3_fs_context_parse_param()` must leave `ctx->symlinkroot` as `NULL` unless `symlinkroot=` is specified, and validate `param->string[0] == '/'` and `strnlen(param->string, PATH_MAX) < PATH_MAX`. [F264]
- `wire_mode_to_posix()` must check `wire_type < ARRAY_SIZE(posix_filetypes)` via `wire_filetype_to_posix()`, and when `wire_type == 0 && is_dir`, set `S_IFDIR` instead of `posix_filetypes[0]` (`S_IFREG`). [F266]
- Minshall+French symlinks (`parse_mf_symlink()`, `format_mf_symlink()`, `couldbe_mf_symlink()`) must have exact size `CIFS_MF_SYMLINK_FILE_SIZE` (1067 bytes) and `link_len <= CIFS_MF_SYMLINK_LINK_MAXLEN` (1024) at offset 43 padded with `'\n'` and `' '` to 1067 bytes. [F269]
- `cifs_ioctl_copychunk()` must verify `dst_file` has `FMODE_WRITE` wrapped in `mnt_want_write_file()`/`mnt_drop_write_file()`, and `src_file` has `f_op->unlocked_ioctl == cifs_ioctl` and is not a directory; `CIFS_DUMP_KEY` and `CIFS_DUMP_FULL_KEY` require `capable(CAP_SYS_ADMIN)`, `smb3_encryption_required(tcon)`, and `out.in_size >= sizeof(out)` plus all three key lengths. [F341, F342]

## Known Pitfalls

- In `cifs_reopen_file()`, `O_CREAT`, `O_EXCL`, and `O_TRUNC` must be masked off from `cfile->f_flags` before `cifs_posix_open()`; in `cifs_nt_open()`, `cifs_reopen_file()`, and `cifs_do_create()`, when an fscache `O_WRONLY` open (`rdwr_for_fscache == 1`) fails with `-EACCES`, retry must strip `GENERIC_READ`, set `rdwr_for_fscache = 2`, and call `cifs_invalidate_cache(inode, FSCACHE_INVAL_DIO_WRITE)` on success. [F277, F287]
- In `cifs_push_mandatory_locks()` and `cifs_unlock_range()`, `cur = buf` and `num = 0` must be initialized at each pass (`i = 0, 1`) and reset whenever `++num == max_num` flushes via `cifs_lockv()`, advancing `cur++` only when not full. [F286]
- In `cifs_sfu_type()`, `IntxLNK\1` SFU symlinks require `fattr->cf_eof > 8` and even, and `symlink_len_utf16` even, in `(0, fattr->cf_eof - 8]`, and free of NULs (`UniStrnlen(..., symlink_len_utf16 / 2) == symlink_len_utf16 / 2`, in 2-byte units) before `cifs_strndup_from_utf16(..., symlink_len_utf16)` (in bytes). [F275, F304]
- In `smb3_fs_context_parse_param()`, empty-capable string options (`"pass"`/`"password"`, `"user"`/`"username"`, `"pass2"`/`"password2"`) must set `skip_parsing = true` when `param->string[0] == 0` before `fs_parse()`. [F282]
- In `cifs_ci_compare()`, because the loop advances `i += l1`, the fallback branch where `char2uni()` returns negative for both characters (`l1 < 0 && l2 < 0`) and raw bytes match must set `l1 = 1` before `continue`. [F290]
