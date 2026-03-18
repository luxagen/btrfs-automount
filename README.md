# BTRam: BTRFS auto-mounter

## What is it?

`btram` was developed as a companion tool to [btrbk](http://github.com/digint/btrbk) for those of us who:
* georeplicate datasets on multiple computers;
* want the datasets to be easily accessible, and up to date, on more than one of those servers.

## What does it do?

`btram` mounts the latest snapshot of each nominated dataset into a given location via a [BTRFS](http://en.wikipedia.org/wiki/Btrfs) subvolume mount, and can automatically remount the latest snapshot each time it's run. For example, if you use `btrbk` to incremental-send updated snapshots each night, invoking `btram -r` will remount the newest snapshot. Because it inspects BTRFS subvolumes and uses [`mount`](http://linux.die.net/man/8/mount), it must be run as ```root```.

## Where can I use it?

### Linux / NAS

It should work on any [*NIX](http://en.wikipedia.org/wiki/Unix-like) system that supports BTRFS. It has been tested on [Ubuntu](http://ubuntu.com/) [GNU](http://www.gnu.org/)/Linux™.

## License

`btram` is released under a "[GPL3](http://www.gnu.org/licenses/gpl-3.0.en.html) or later" licence and, despite my best efforts to make it obviously defect-free, comes with absolutely no warranty whatsoever, express or implied; use it at your own risk. Feel free to log [issues](http://github.com/luxagen/btram/issues) for bug reports and suggestions, submit a pull request with a fix of your own, or e-mail me at btram@luxagen.com .

## Installation

1. Install Perl.
2. Clone or download this repository.
3. Install the dependencies:

	Ubuntu:
	```
	sudo apt install libipc-run-perl libgetopt-lucid-perl
	```
	CPAN:
	```
	cpan -I IPC::Run Getopt::Lucid
	```

### Add to PATH (optional)

To facilitate addition to your `$PATH`, a `bin/` directory is provided with a convenience symlink.

## Configuration

`btram`'s configuration file must exist at `/etc/btram.conf` in order to work. This file consists of one line per subvolume mount, with 4 tab-separated columns. Multiple consecutive tabs count as a single column separator. The four columns are:

```
<mountpoint>	<btrfs-vol-root>	<snapshot-dir>	<snapshot-prefix>
```

`<btrfs-vol-root>` is the filesystem location where the root subvolume is mounted; `btram` requires the root subvolume to be mounted and snapshots to live somewhere in that root subvolume. `<snapshot-dir>` is the relative path from the root mountpoint to the directory containing the snapshots, and `<snapshot-prefix>` is the equivalent of `btrbk`'s `snapshot_name` directive, i.e. the portion of the filename preceding the `.$datestamp` suffix.

### Example

```
/My Data	/btr/a	.btrbk/server2	my-data
```

## How to use

```
btram -h
```

...will tell you how to use the tool. Note that it won't actually do any [re]mounting unless you invoke it with the ```-r``` option. A ```--verbose``` option exists that will print more diagnostic information, including the commands that `btram -r` will run.

## Exit codes

This section documents the *planned* (proposed) exit code scheme for `btram`, intended to allow safe external iteration (e.g. a `bash` `while` loop) and better diagnostics.

The overall exit code is the maximum-severity value observed during a run (even though `btram` may continue attempting other roots after encountering an error).

| Code | Meaning |
| ---: | --- |
| 0 | No changes required (nothing to do), and no retryable blockers detected |
| 1 | Would perform changes (dry-run) / performed at least one change (run) |
| 2 | Retryable blocked: a condition that may succeed on a subsequent invocation |
| 65 (`EX_DATAERR`) | Config line malformed or missing required fields |
| 66 (`EX_NOINPUT`) | Missing/inaccessible `/etc/btram.conf` |
| 70 (`EX_SOFTWARE`) | Internal error / invariant violation |
| 78 (`EX_CONFIG`) | Configuration violates policy (e.g. mountpoints under ignored mount trees) |
| 80 (`BTRAM_BIND_BLOCK`) | Transaction blocked due to a bind mount (ancestor or descendant) |
| 81 (`BTRAM_UMOUNT_FAILED`) | `umount` failed during a transaction |
| 82 (`BTRAM_MOUNT_FAILED`) | `mount`/remount failed for a root swap or config-driven mount |
| 83 (`BTRAM_RESTORE_FAILED`) | Restoring descendant mounts failed |
| 84 (`BTRAM_ROLLBACK_FAILED`) | Rollback failed after a swap failure |

### Example: external convergence loop

The following pattern reruns `btram` only when it exits with code `2` (retryable blocked):

```bash
while true; do
  btram -r
  rc=$?
  [ $rc -eq 2 ] || exit $rc
  sleep 1
done