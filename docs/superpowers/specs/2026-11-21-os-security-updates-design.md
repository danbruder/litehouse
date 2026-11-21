# Automatic OS Security Updates — Design

## Scope

"Updates" here means **OS security patches on the litehouse host** (the
Ubuntu/Debian droplet). Updating litehouse itself is separate (deploy-on-main /
`lh upgrade`).

## What gets configured

`lh install` (phase 14), every `lh upgrade`, and `lh os-updates enable` run the
same idempotent script (`templates::os_security_updates_script`):

1. Install `unattended-upgrades` if missing (waits up to 300s for the apt lock).
2. `/etc/apt/apt.conf.d/20auto-upgrades` — periodic list refresh, download,
   and unattended-upgrade every day (`APT::Periodic::*`).
3. `/etc/apt/apt.conf.d/52litehouse-unattended-upgrades` — overrides on top of
   the distro's `50unattended-upgrades`:
   - **Allowed origins left at the distro default** (the `-security` pocket), so
     only security fixes land unattended — no feature upgrades.
   - `Automatic-Reboot "false"`. The existing weekly reboot timer
     (`litehouse-weekly-reboot.timer`, Sun 03:00 America/New_York; see
     2026-07-16-server-hardening-design.md §3) is what activates a patched
     kernel/libc. A second, unscheduled reboot window would just add outages.
   - Remove unused kernels/dependencies so `/boot` and disk don't fill up.
4. Drop-in `/etc/systemd/system/apt-daily-upgrade.timer.d/litehouse.conf` moving
   the upgrade run to **02:00 America/New_York** (±15 min). The distro default is
   06:00 host time (UTC on a droplet), i.e. US daytime. A `docker.io` /
   `containerd` security update restarts dockerd and therefore every container;
   they come back via their restart policy, but that should happen in the quiet
   window, before the 03:00 nightly app restart and the Sunday reboot (so the
   reboot picks up whatever was just installed).

## Checking a server

`lh os-updates status [--json]` (on the host, no root needed) reports whether
the package is installed, the `APT::Periodic` switches, the timer and its next
run, the reboot policy, whether a reboot is pending (`/var/run/reboot-required`
and the packages that asked for it), the next weekly reboot, and the last
unattended-upgrades log line. Exits 1 if updates aren't fully enabled.

Existing servers pick this up on their next `lh upgrade`; a failure there is a
warning (the container upgrade already succeeded) and `lh os-updates enable`
retries it.

## Non-goals

- Non-security package upgrades, distro release upgrades.
- Rebooting outside the weekly window. If a critical kernel fix can't wait for
  Sunday, reboot by hand; every container restarts on its own.
